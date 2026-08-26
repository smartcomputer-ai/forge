import { and, eq } from "drizzle-orm";
import type { Client } from "@temporalio/client";
import { LightspeedClient } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import {
  admitTriggerEvent,
  checkTriggerBreaker,
  storeBotEvent,
  type AdmissionDeps,
} from "@lightspeed/bots/admission";
import {
  chatMessageEventId,
  chatSentEventId,
  type BotEventDocumentV1,
} from "@lightspeed/bots/contracts";
import type {
  BotBridgeActivities,
  ChatHandleV1,
  EmitChatEventInput,
  StoreChatSentInput,
} from "../contracts/bridge.js";
import { mediaLabel } from "../contracts/media.js";
import { formatMessageLine } from "../policy/activation.js";

/**
 * Channels as a bot event source. Every message goes through the shared
 * admission pipeline of the bot's `chat` trigger (breaker, filter, route,
 * coalesce, delivery policy, store-then-wake); the bot's own sends are
 * archived beside them so a message number means the same thing in both
 * directions.
 */

export interface BotBridgeConfig {
  db: Db;
  endpoint: string;
  temporal: Client;
  fetch?: typeof fetch;
}

export const CHAT_MESSAGE_KIND = "chat.message";
export const CHAT_SENT_KIND = "chat.sent";
/** The summary is the model-facing line; a very long message continues via bot_event_read. */
const SUMMARY_CAP = 2_000;

export function createBotBridgeActivities(config: BotBridgeConfig): BotBridgeActivities {
  const clientFor = (universeId: string) =>
    new LightspeedClient({
      endpoint: config.endpoint,
      ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
      headers: { "x-lightspeed-universe": universeId },
    });
  const depsFor = (universeId: string): AdmissionDeps => ({
    db: config.db,
    temporal: config.temporal,
    engine: clientFor(universeId),
  });

  async function loadTrigger(triggerId: string, botId: string) {
    const [row] = await config.db
      .select({ trigger: schema.botTriggers, bot: schema.bots })
      .from(schema.botTriggers)
      .innerJoin(schema.bots, eq(schema.bots.id, schema.botTriggers.botId))
      .where(and(eq(schema.botTriggers.id, triggerId), eq(schema.botTriggers.botId, botId)))
      .limit(1);
    return row ?? null;
  }

  return {
    async emitChatEvent(input) {
      const loaded = await loadTrigger(input.triggerId, input.botId);
      if (loaded === null || !loaded.trigger.enabled) return { status: "refused", reason: "trigger_disabled" };
      if (!loaded.bot.enabled) return { status: "refused", reason: "bot_disabled" };
      const { trigger, bot } = loaded;
      const breaker = await checkTriggerBreaker({ db: config.db }, bot, trigger);
      if (breaker.tripped) return { status: "refused", reason: "breaker_tripped" };

      const document = chatMessageDocument(input);
      const eventId = chatMessageEventId(trigger.id, input.conversation.key, input.message.messageId);
      const admitted = await admitTriggerEvent(depsFor(input.universeId), {
        bot,
        trigger,
        universeId: input.universeId,
        eventId,
        document,
        // The summary already is the message line; attachments are labelled
        // below it and reach the model as run input items.
        ...(input.media.length === 0
          ? {}
          : {
              promptData: input.media.map((item) =>
                mediaLabel({ kind: item.kind, ...(item.name == null ? {} : { name: item.name }) }),
              ),
            }),
        ...(input.media.length === 0
          ? {}
          : {
              media: input.media.map((item) => ({
                blobRef: item.blobRef,
                kind: item.kind,
                mime: item.mime,
                ...(item.name == null ? {} : { name: item.name }),
              })),
            }),
        tools: input.toolsRef,
        notify: {
          workflowId: input.notify.workflowId,
          workflowKind: input.notify.workflowKind,
          token: input.notify.token,
        },
      });
      return {
        status: admitted.archived ? "archived" : admitted.duplicate ? "duplicate" : "admitted",
        eventId,
        seq: admitted.event.seq ?? null,
        sessionId: admitted.event.session?.sessionId ?? null,
      };
    },

    async storeChatSent(input) {
      const loaded = await loadTrigger(input.triggerId, input.botId);
      if (loaded === null) return { seq: null };
      const document = chatSentDocument(input);
      const { event } = await storeBotEvent(depsFor(input.universeId), {
        bot: loaded.bot,
        universeId: input.universeId,
        eventId: chatSentEventId(input.triggerId, input.invocationId),
        document,
        triggerId: input.triggerId,
        // Archived: the send is already in the session as the tool call; the
        // row exists so the number resolves and the log reads as the chat.
        deliver: false,
      });
      return { seq: event.seq ?? null };
    },

    async resolveChatHandle(input) {
      const [bot] = await config.db
        .select({ eventSeq: schema.bots.eventSeq })
        .from(schema.bots)
        .where(eq(schema.bots.id, input.botId))
        .limit(1);
      const maxSeq = bot?.eventSeq ?? 0;
      const [row] = await config.db
        .select()
        .from(schema.botEvents)
        .where(and(eq(schema.botEvents.botId, input.botId), eq(schema.botEvents.seq, input.seq)))
        .limit(1);
      if (!row || (row.kind !== CHAT_MESSAGE_KIND && row.kind !== CHAT_SENT_KIND)) {
        return { handle: null, maxSeq };
      }
      const response = await clientFor(input.universeId).call("blobs/read", { blobRef: row.ref });
      const document = JSON.parse(
        Buffer.from(response.result.bytesBase64, "base64").toString("utf8"),
      ) as BotEventDocumentV1;
      const handle = handleFromDocument(document, input.conversationKey);
      return { handle, maxSeq };
    },
  };
}

/** The stored envelope of one inbound message; `data` keeps the provider ids the model never sees. */
export function chatMessageDocument(input: EmitChatEventInput): BotEventDocumentV1 {
  const { conversation, message } = input;
  const line = formatMessageLine(message, message.text);
  return {
    version: 1,
    kind: CHAT_MESSAGE_KIND,
    source: `${conversation.route.provider}:${conversation.route.accountId}`,
    occurredAt: new Date(message.timestampMs).toISOString(),
    summary: line.length > SUMMARY_CAP ? `${line.slice(0, SUMMARY_CAP)}… (full text via bot_event_read)` : line,
    data: {
      conversation: {
        key: conversation.key,
        label: conversation.label,
        scope: conversation.scope,
        provider: conversation.route.provider,
        chatId: conversation.route.chatId,
        ...(conversation.route.threadId === undefined ? {} : { threadId: conversation.route.threadId }),
      },
      sender: { id: message.senderId, name: message.senderName, memberRole: message.memberRole },
      messageId: message.messageId,
      text: message.text,
      isDirect: message.isDirect,
      mentionedBot: message.mentionedBot,
      isReplyToBot: message.isReplyToBot,
      ...(input.media.length === 0
        ? {}
        : {
            media: input.media.map((item) => ({
              kind: item.kind,
              mime: item.mime,
              ...(item.name == null ? {} : { name: item.name }),
            })),
          }),
    },
  };
}

/** The archived envelope of one of the bot's own sends. */
export function chatSentDocument(input: StoreChatSentInput): BotEventDocumentV1 {
  const { conversation } = input;
  const line = `sent: ${input.text}`;
  return {
    version: 1,
    kind: CHAT_SENT_KIND,
    source: `${conversation.route.provider}:${conversation.route.accountId}`,
    occurredAt: new Date().toISOString(),
    summary: line.length > SUMMARY_CAP ? `${line.slice(0, SUMMARY_CAP)}…` : line,
    data: {
      conversation: {
        key: conversation.key,
        label: conversation.label,
        provider: conversation.route.provider,
        chatId: conversation.route.chatId,
        ...(conversation.route.threadId === undefined ? {} : { threadId: conversation.route.threadId }),
      },
      text: input.text,
      providerMessageIds: input.providerMessageIds,
      fromMe: true,
      ...(input.replyTo === null ? {} : { replyTo: input.replyTo }),
    },
  };
}

/** The handle behind a stored chat row, or null when it belongs to another conversation. */
export function handleFromDocument(
  document: BotEventDocumentV1,
  conversationKey: string,
): ChatHandleV1 | null {
  const data = document.data as Record<string, unknown> | undefined;
  const conversation = data?.conversation as { key?: unknown } | undefined;
  if (conversation?.key !== conversationKey) return null;
  if (document.kind === CHAT_SENT_KIND) {
    const ids = Array.isArray(data?.providerMessageIds)
      ? data.providerMessageIds.filter((id): id is string => typeof id === "string" && id.length > 0)
      : [];
    if (ids.length === 0) return null;
    return {
      providerMessageIds: ids,
      fromMe: true,
      ...(typeof data?.text === "string" ? { text: data.text } : {}),
    };
  }
  if (document.kind === CHAT_MESSAGE_KIND) {
    const messageId = data?.messageId;
    if (typeof messageId !== "string" || messageId.length === 0) return null;
    const sender = data?.sender as { id?: unknown } | undefined;
    return {
      providerMessageIds: [messageId],
      fromMe: false,
      ...(typeof sender?.id === "string" ? { senderId: sender.id } : {}),
      ...(typeof data?.text === "string" ? { text: data.text } : {}),
    };
  }
  return null;
}
