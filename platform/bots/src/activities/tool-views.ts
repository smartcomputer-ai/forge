import type { schema } from "@lightspeed/platform-db";
import type { BotRow, BotTriggerRow } from "../config.js";
import type { BotEventDocumentV1 } from "../contracts/bots.js";

/**
 * Model-facing shapes of the `bot_*` tool results.
 *
 * The rule: the model reads and echoes `#N` and labels; digest ids, uuids,
 * and session ids stay in rows and on the platform API. Every shape is a
 * pure function here so the guarantee is tested without a database, and a
 * new field cannot quietly bring a digest back.
 */

export type BotEventRow = typeof schema.botEvents.$inferSelect;

/** Controller-side state the tool activity cannot read from the database. */
export interface BotControllerSummary {
  sessions: { label: string; kind: string }[];
  /** Active deliveries as the `#N`s they carry and the session label they run in. */
  activeDeliveries: { events: number[]; session: string }[];
  /** Coalescing buffers by the session label they will deliver to. */
  buffers: { session: string; count: number; flushAtMs: number }[];
  runsToday: number;
  eventsProcessed: number;
  /**
   * Federation context of the invoking session — the hop count of the
   * delivery it is handling and its logical route for receipts. Read by
   * `bot_emit`; never shown to the model.
   */
  invocation: { hops: number; session?: { sessionId: string; label: string } };
}

export function botStatusView(bot: BotRow, controller: BotControllerSummary) {
  return {
    bot: {
      botId: bot.name,
      displayName: bot.displayName,
      description: bot.description,
      enabled: bot.enabled,
      brief: bot.brief,
      runsPerDay: bot.runsPerDay,
      runsToday: controller.runsToday,
      breaker: bot.breaker,
      routedSessionTtlMs: bot.routedSessionTtlMs,
      selfConfig: bot.selfConfig,
      emit: bot.emit,
      eventsProcessed: controller.eventsProcessed,
    },
    sessions: controller.sessions,
    activeDeliveries: controller.activeDeliveries,
    buffers: controller.buffers,
  };
}

/** How a chat trigger's account is named to the model: `provider:accountId`, never the row key. */
export interface ChatAccountLabel {
  provider: string;
  accountId: string;
}

export function channelAccountHandle(account: ChatAccountLabel): string {
  return `${account.provider}:${account.accountId}`;
}

/**
 * A trigger by its name; the caller passes the already-redacted row. A chat
 * trigger's spec shows the account as `provider:accountId` (the same handle
 * `bot_trigger_put` accepts) in place of the platform row key.
 */
export function triggerToolView(
  trigger: BotTriggerRow,
  ingestUrl: string | null,
  chatAccount?: ChatAccountLabel | null,
) {
  const spec =
    trigger.kind === "chat"
      ? (() => {
          const { channelAccountId: _channelAccountId, ...rest } = trigger.spec as {
            channelAccountId: string;
          } & Record<string, unknown>;
          return { channelAccount: chatAccount ? channelAccountHandle(chatAccount) : null, ...rest };
        })()
      : trigger.spec;
  return {
    name: trigger.name,
    kind: trigger.kind,
    spec,
    filter: trigger.filter,
    route: trigger.route,
    coalesce: trigger.coalesce,
    deliver: trigger.deliver,
    ...(trigger.sessionTtlMs === null ? {} : { sessionTtlMs: trigger.sessionTtlMs }),
    enabled: trigger.enabled,
    ...(ingestUrl === null ? {} : { ingestUrl }),
  };
}

function sessionLabel(row: Pick<BotEventRow, "session">): { session?: string } {
  return row.session === null ? {} : { session: row.session.label };
}

export function eventListRowView(row: BotEventRow, document: BotEventDocumentV1 | null) {
  return {
    ...(row.seq === null ? {} : { seq: row.seq }),
    kind: row.kind,
    source: row.source,
    occurredAt: row.occurredAt.toISOString(),
    receivedAt: row.receivedAt.toISOString(),
    ...sessionLabel(row),
    summary: document?.summary ?? null,
  };
}

export function filterResultView(
  row: BotEventRow,
  document: BotEventDocumentV1 | null,
  outcome: { matched: boolean; error?: string },
) {
  return {
    ...(row.seq === null ? {} : { seq: row.seq }),
    kind: row.kind,
    source: row.source,
    summary: document?.summary ?? null,
    matched: outcome.matched,
    ...(outcome.error === undefined ? {} : { error: outcome.error }),
  };
}

/** The full archived envelope behind `bot_event_read #N`. */
export function eventEnvelopeView(
  row: BotEventRow,
  document: BotEventDocumentV1,
): Record<string, unknown> {
  return {
    seq: row.seq,
    kind: row.kind,
    source: row.source,
    occurredAt: row.occurredAt.toISOString(),
    receivedAt: row.receivedAt.toISOString(),
    ...sessionLabel(row),
    summary: document.summary,
    ...(document.correlationId == null ? {} : { correlationId: document.correlationId }),
    ...(document.links === undefined ? {} : { links: document.links }),
    ...(document.data === undefined ? {} : { data: document.data }),
    ...(document.headers === undefined ? {} : { headers: document.headers }),
  };
}
