import { and, eq, inArray, isNotNull } from "drizzle-orm";
import { LightspeedClient } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import type { Client } from "@temporalio/client";
import { BOT_DELIVERY_SIGNAL, type BotDeliveryReceiptV1 } from "../contracts/bots.js";
import {
  BOT_DIRECTORY_KEY,
  BOT_DIRECTORY_TITLE,
  BotAdmissionRefusal,
  directoryEntriesFor,
  nextHops,
  receiptDocument,
  receiptEventId,
  recordBotActivity,
  renderBotDirectory,
  storeBotEvent,
  type AdmissionDeps,
} from "../admission.js";

/**
 * Federation activities: the directory a sender reads, and the receipts a
 * receiver's controller sends when a delivery that carried an ask finishes.
 * Both are best effort from the controller's point of view — a failure
 * costs a stale directory or a missing receipt, never a delivery.
 */

export interface BotFederationConfig {
  db: Db;
  endpoint: string;
  temporal: Client;
  fetch?: typeof fetch;
}

export interface PublishBotDirectoryInput {
  /** Lightspeed universe id. */
  universeId: string;
  /** Bot row key. */
  botId: string;
  sessionId: string;
}

export interface SendBotReceiptsInput {
  /** Lightspeed universe id. */
  universeId: string;
  /** Row key of the bot whose delivery finished. */
  botId: string;
  deliveryId: string;
  /** Event ids in the delivery that asked for a receipt. */
  eventIds: string[];
  status: string;
  summary: string | null;
  /** Highest hop count in the finished delivery. */
  hops: number;
}

export interface SendDeliveryReceiptsInput {
  /** Row key of the bot whose delivery changed state. */
  botId: string;
  /** Event ids in the delivery; only rows with a `notify` route are signalled. */
  eventIds: string[];
  receipt: Omit<BotDeliveryReceiptV1, "version" | "token">;
}

export interface BotFederationActivities {
  /**
   * Put the `bot:directory` catalog into a session before a delivery: the
   * bots whose inbox accepts this one. A same-content put is a no-op in the
   * engine, so calling it before every delivery is cheap and keeps the
   * prefix cache intact (P136).
   */
  publishBotDirectory(input: PublishBotDirectoryInput): Promise<{ entries: number }>;
  /** Admit one `bot.reply` receipt into each asking bot's session. */
  sendBotReceipts(input: SendBotReceiptsInput): Promise<{ sent: number }>;
  /**
   * Signal `bot_delivery_v1` to every admitting source that asked for
   * receipts on the delivery's events — one signal per (workflow, token).
   * A source that is gone is skipped; the delivery never waits on it.
   */
  sendDeliveryReceipts(input: SendDeliveryReceiptsInput): Promise<{ sent: number; skipped: number }>;
}

export function createBotFederationActivities(config: BotFederationConfig): BotFederationActivities {
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

  return {
    async publishBotDirectory(input) {
      const [me] = await config.db
        .select()
        .from(schema.bots)
        .where(eq(schema.bots.id, input.botId))
        .limit(1);
      if (!me) throw new Error("bot not found");
      const rows = await config.db
        .select({ bot: schema.bots, inbox: schema.botTriggers })
        .from(schema.bots)
        .leftJoin(
          schema.botTriggers,
          and(eq(schema.botTriggers.botId, schema.bots.id), eq(schema.botTriggers.kind, "bot")),
        )
        .where(eq(schema.bots.universeId, me.universeId));
      const entries = directoryEntriesFor(
        me.name,
        rows.map(({ bot, inbox }) => ({
          name: bot.name,
          enabled: bot.enabled,
          description: bot.description,
          inbox: inbox === null ? null : { enabled: inbox.enabled, spec: inbox.spec },
        })),
      );
      await clientFor(input.universeId).call("session/context/append", {
        sessionId: input.sessionId,
        entries: [
          {
            key: BOT_DIRECTORY_KEY,
            item: { type: "catalog", title: BOT_DIRECTORY_TITLE, text: renderBotDirectory(entries) },
          },
        ],
      });
      return { entries: entries.length };
    },

    async sendBotReceipts(input) {
      if (input.eventIds.length === 0) return { sent: 0 };
      const [answering] = await config.db
        .select()
        .from(schema.bots)
        .where(eq(schema.bots.id, input.botId))
        .limit(1);
      if (!answering) throw new Error("bot not found");
      const asked = await config.db
        .select()
        .from(schema.botEvents)
        .where(
          and(
            eq(schema.botEvents.botId, answering.id),
            inArray(schema.botEvents.eventId, input.eventIds),
            isNotNull(schema.botEvents.replyTo),
          ),
        );
      let hops: number;
      try {
        hops = nextHops(input.hops);
      } catch (error) {
        if (error instanceof BotAdmissionRefusal) {
          await recordBotActivity(config.db, answering.id, "loop_cut", {
            eventId: input.deliveryId,
            detail: `receipts for delivery not sent: ${error.message}`,
          });
          return { sent: 0 };
        }
        throw error;
      }
      const deps = depsFor(input.universeId);
      let sent = 0;
      for (const row of asked) {
        const route = row.replyTo;
        if (route === null || row.seq === null) continue;
        const [asker] = await config.db
          .select()
          .from(schema.bots)
          .where(eq(schema.bots.id, route.botId))
          .limit(1);
        if (!asker || !asker.enabled) continue;
        const document = receiptDocument({
          answering: answering.name,
          askedSeq: row.seq,
          status: input.status,
          summary: input.summary,
          occurredAt: new Date().toISOString(),
          hops,
        });
        const { event } = await storeBotEvent(deps, {
          bot: asker,
          universeId: input.universeId,
          eventId: receiptEventId(answering.id, input.deliveryId, row.eventId),
          document,
          ...(route.session === undefined ? {} : { session: route.session }),
          whenBusy: "queue",
          senderBotId: answering.id,
          hops,
          inReplyTo: { bot: answering.name, seq: row.seq },
        });
        await recordBotActivity(config.db, answering.id, "replied", {
          eventId: row.eventId,
          detail: `replied ${input.status} to ${asker.name} (#${event.seq ?? "?"} there)`,
        });
        sent += 1;
      }
      return { sent };
    },

    async sendDeliveryReceipts(input) {
      if (input.eventIds.length === 0) return { sent: 0, skipped: 0 };
      const rows = await config.db
        .select({ notify: schema.botEvents.notify })
        .from(schema.botEvents)
        .where(
          and(
            eq(schema.botEvents.botId, input.botId),
            inArray(schema.botEvents.eventId, input.eventIds),
            isNotNull(schema.botEvents.notify),
          ),
        );
      const targets = new Map<string, { workflowId: string; token: string }>();
      for (const row of rows) {
        if (row.notify === null) continue;
        targets.set(`${row.notify.workflowId}\n${row.notify.token}`, {
          workflowId: row.notify.workflowId,
          token: row.notify.token,
        });
      }
      let sent = 0;
      let skipped = 0;
      for (const target of targets.values()) {
        const receipt: BotDeliveryReceiptV1 = { version: 1, token: target.token, ...input.receipt };
        try {
          await config.temporal.workflow.getHandle(target.workflowId).signal(BOT_DELIVERY_SIGNAL, receipt);
          sent += 1;
        } catch {
          skipped += 1;
        }
      }
      return { sent, skipped };
    },
  };
}
