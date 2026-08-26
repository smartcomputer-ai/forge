import type { ChannelRoute } from "./channel.js";
import type { ChannelInputItem } from "./media.js";

/**
 * The conversation workflow's two links to the bot: events go in through
 * admission (`emitChatEvent`), the bot's own sends are archived beside them
 * (`storeChatSent`), and message numbers resolve back to provider ids
 * (`resolveChatHandle`). Plus the core-client helpers the workflow keeps.
 */

export interface WorkflowEndpointV1 {
  workflowId: string;
  workflowKind: string;
}

export interface ReadJsonBlobInput {
  universeId: string;
  blobRef: string;
}

export interface PutJsonBlobInput {
  universeId: string;
  value: unknown;
}

export interface PutJsonBlobResult {
  blobRef: string;
}

export interface PutChatToolDeclarationsInput {
  universeId: string;
  /** This conversation workflow: the receiver every `message_*` call is pushed to. */
  receiver: WorkflowEndpointV1;
}

export interface PutChatToolDeclarationsResult {
  /** CAS ref of the declaration array; content-addressed, so stable per receiver. */
  toolsRef: string;
  toolIds: string[];
}

export interface ReconcileDeliveryInput {
  universeId: string;
  sessionId: string;
  runId: string | null;
  /** The bot lane's finish status (`handled`, `run_failed`, `unresolved`, …). */
  status: string;
}

export type ReconcileDeliveryResult =
  | { action: "suppress"; reason: "messaging_tool" | "no_run" }
  | { action: "deliver"; text: string };

export interface LightspeedActivities {
  readJsonBlob(input: ReadJsonBlobInput): Promise<unknown>;
  putJsonBlob(input: PutJsonBlobInput): Promise<PutJsonBlobResult>;
  putChatToolDeclarations(input: PutChatToolDeclarationsInput): Promise<PutChatToolDeclarationsResult>;
  /**
   * After the bot's delivery finished: nothing to do when the run answered
   * through a `message_*` tool, otherwise the assistant's final text (or a
   * failure line) to send as the reply.
   */
  reconcileDelivery(input: ReconcileDeliveryInput): Promise<ReconcileDeliveryResult>;
}

export interface EmitChatEventInput {
  universeId: string;
  triggerId: string;
  botId: string;
  conversation: { key: string; label: string; scope: "direct" | "group"; route: ChannelRoute };
  message: {
    messageId: string;
    senderId: string;
    senderName: string;
    memberRole: "member" | "admin" | "owner" | null;
    timestampMs: number;
    /** The activated text (prefix or mention stripped). */
    text: string;
    isDirect: boolean;
    mentionedBot: boolean;
    isReplyToBot: boolean;
  };
  media: Array<Extract<ChannelInputItem, { type: "media" }>>;
  toolsRef: string;
  notify: WorkflowEndpointV1 & { token: string };
}

export type EmitChatEventResult =
  | {
      status: "admitted" | "archived" | "duplicate";
      eventId: string;
      seq: number | null;
      /** The routed session the event was admitted to (logical base id). */
      sessionId: string | null;
    }
  | { status: "refused"; reason: "breaker_tripped" | "trigger_disabled" | "bot_disabled" };

export interface StoreChatSentInput {
  universeId: string;
  triggerId: string;
  botId: string;
  conversation: { key: string; label: string; route: ChannelRoute };
  /** Stable across retries: the invocation id, or `fallback:<deliveryId>`. */
  invocationId: string;
  text: string;
  providerMessageIds: string[];
  /** The message number this send replied to, if any. */
  replyTo: number | null;
}

export interface StoreChatSentResult {
  seq: number | null;
}

export interface ChatHandleV1 {
  /** Every provider id the message occupies (a chunked send has several); the first is the anchor. */
  providerMessageIds: string[];
  fromMe: boolean;
  senderId?: string;
  text?: string;
}

export interface ResolveChatHandleInput {
  universeId: string;
  botId: string;
  /** The conversation the handle must belong to; a number from another chat is unknown here. */
  conversationKey: string;
  seq: number;
}

export interface ResolveChatHandleResult {
  handle: ChatHandleV1 | null;
  /** The bot's event numbers run 1..max; for the "unknown #N" error. */
  maxSeq: number;
}

export interface BotBridgeActivities {
  emitChatEvent(input: EmitChatEventInput): Promise<EmitChatEventResult>;
  storeChatSent(input: StoreChatSentInput): Promise<StoreChatSentResult>;
  resolveChatHandle(input: ResolveChatHandleInput): Promise<ResolveChatHandleResult>;
}
