import { createHash } from "node:crypto";
import { and, count, eq, gte, inArray, isNull } from "drizzle-orm";
import type { LightspeedClient } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import type { BotRow, BotTriggerRow } from "./config.js";
import {
  MAX_BOT_HOPS,
  type BotCoalesceParamsV1,
  type BotEvent,
  type BotEventDocumentV1,
  type BotEventFinalOutcome,
  type BotEventMediaV1,
  type BotEventNotifyV1,
  type BotEventSession,
  type BotInboxTriggerSpecV1,
  type BotStartV1,
  type BotWhenBusyV1,
} from "./contracts/bots.js";
import { allocateBotEventSeq, renderAdmittedEvent, wakeBotController, type BotWakeClient } from "./events.js";
import { computeRouteSession, evaluateFilter, type FilterContext } from "./webhooks.js";

/**
 * Event admission, shared by every path that puts an event in front of a
 * bot: webhook ingest, polls, schedules, self-emits, addressed emits, and
 * receipts. One pipeline — breaker, filter, route, coalesce, delivery
 * policy, store-then-wake — so a bot's inbox behaves the same whoever is
 * knocking. Refusals are typed so the model can read why an emit failed.
 */

export type BotEventRow = typeof schema.botEvents.$inferSelect;
type PersistedBotEventNotify = NonNullable<BotEventRow["notify"]>;

const NOTIFY_TOKEN_ENCODING = "base64url-v1" as const;

/**
 * PostgreSQL jsonb rejects U+0000 even when JSON.stringify escapes it. Receipt
 * tokens are caller-owned opaque bytes-in-a-string, so encode them at the
 * persistence boundary and decode them before signalling the caller back.
 */
export function persistBotEventNotify(notify: BotEventNotifyV1): PersistedBotEventNotify {
  return {
    workflowId: notify.workflowId,
    workflowKind: notify.workflowKind,
    token: Buffer.from(notify.token, "utf8").toString("base64url"),
    tokenEncoding: NOTIFY_TOKEN_ENCODING,
  };
}

/** Read both encoded rows and legacy rows whose tokens were already jsonb-safe. */
export function restoreBotEventNotifyToken(notify: PersistedBotEventNotify): string {
  return notify.tokenEncoding === NOTIFY_TOKEN_ENCODING
    ? Buffer.from(notify.token, "base64url").toString("utf8")
    : notify.token;
}

export interface AdmissionDeps {
  db: Db;
  temporal: BotWakeClient;
  /** Core client for the receiving universe (CAS puts). */
  engine: Pick<LightspeedClient, "call">;
}

export const BOT_REFUSAL_CODES = [
  "unknown_bot",
  "bot_disabled",
  /** Terminal, unlike `bot_disabled`: the target was closed and will not come back. */
  "bot_closed",
  "no_inbox",
  "not_accepted",
  "filtered",
  "breaker_tripped",
  "rate_limited",
  "loop_cut",
] as const;
export type BotRefusalCode = (typeof BOT_REFUSAL_CODES)[number];

/** Why an emit was refused; `code` is stable, `message` is for the model. */
export class BotAdmissionRefusal extends Error {
  constructor(
    readonly code: BotRefusalCode,
    message: string,
  ) {
    super(message);
    this.name = "BotAdmissionRefusal";
  }
}

/** Controller start config for a bot row; the same shape every admission path signals. */
export function botStartFor(bot: BotRow, universeId: string): BotStartV1 {
  return {
    version: 1,
    universeId,
    botId: bot.id,
    botName: bot.name,
    displayName: bot.displayName,
    profileId: bot.profileId,
    brief: bot.brief,
    runsPerDay: bot.runsPerDay,
    routedSessionTtlMs: bot.routedSessionTtlMs,
    selfConfig: bot.selfConfig,
    emit: bot.emit,
    enabled: bot.enabled,
    ...(bot.closedAt === null ? {} : { closed: true }),
  };
}

function digest(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

/**
 * Per-receiver id of a bot-originated event: the tool invocation id is
 * stable across activity retries, so a retried emit converges on one row.
 * One inbox per bot keeps it unique per receiver.
 */
export function botEventIdFor(senderBotId: string, invocationId: string): string {
  return `bot:${senderBotId}:${digest(invocationId)}`;
}

/** Id of the receipt for one asked event of one finished delivery. */
export function receiptEventId(answeringBotId: string, deliveryId: string, eventId: string): string {
  return `reply:${answeringBotId}:${digest(`${deliveryId}\n${eventId}`)}`;
}

/** The hop count of an event caused by a delivery at `causingHops`, or a `loop_cut` refusal. */
export function nextHops(causingHops: number): number {
  const hops = causingHops + 1;
  if (hops > MAX_BOT_HOPS) {
    throw new BotAdmissionRefusal(
      "loop_cut",
      `this exchange has crossed ${MAX_BOT_HOPS} bots without reaching the world; not forwarding further`,
    );
  }
  return hops;
}

/** The highest hop count among a delivery's events (0 when none carry one). */
export function deliveryHops(events: Pick<BotEvent, "hops">[]): number {
  return events.reduce((max, event) => Math.max(max, event.hops ?? 0), 0);
}

export interface InboxTarget {
  bot: Pick<BotRow, "name" | "enabled" | "closedAt">;
  /** The target's `bot`-kind trigger, if any. */
  inbox: BotTriggerRow | null;
}

/**
 * Resolve the inbox an addressed emit goes through: the target must exist,
 * be open and enabled, declare an enabled inbox, and list the sender (or
 * nobody). A closed bot is refused with its own code so a sending model
 * stops retrying.
 */
export function resolveInbox(
  sender: Pick<BotRow, "name">,
  targetName: string,
  target: InboxTarget | null,
): BotTriggerRow {
  if (target === null) {
    throw new BotAdmissionRefusal("unknown_bot", `no bot named ${targetName} in this universe`);
  }
  if (target.bot.closedAt !== null) {
    throw new BotAdmissionRefusal("bot_closed", `${targetName} was closed and no longer accepts events`);
  }
  if (!target.bot.enabled) {
    throw new BotAdmissionRefusal("bot_disabled", `${targetName} is disabled`);
  }
  if (target.inbox === null || target.inbox.kind !== "bot" || !target.inbox.enabled) {
    throw new BotAdmissionRefusal(
      "no_inbox",
      `${targetName} has no enabled inbox (a trigger of kind bot) for events from other bots`,
    );
  }
  const spec = target.inbox.spec as BotInboxTriggerSpecV1;
  if (spec.from !== undefined && !spec.from.includes(sender.name)) {
    throw new BotAdmissionRefusal(
      "not_accepted",
      `${targetName}'s inbox does not accept events from ${sender.name}`,
    );
  }
  return target.inbox;
}

export interface EventOutcomeInput {
  outcome: BotEventFinalOutcome;
  detail?: string | null;
  deliveryId?: string | null;
  runId?: string | null;
}

/**
 * Write-once outcome on event rows: the read model of what the controller
 * decided (the decision itself is in its Temporal history). A row already
 * resolved keeps its first outcome; a retried write is a no-op.
 */
export async function recordEventOutcomes(
  db: Db,
  botId: string,
  eventIds: string[],
  input: EventOutcomeInput,
): Promise<{ updated: number }> {
  if (eventIds.length === 0) return { updated: 0 };
  const rows = await db
    .update(schema.botEvents)
    .set({
      outcome: input.outcome,
      outcomeDetail: input.detail ?? null,
      deliveryId: input.deliveryId ?? null,
      runId: input.runId ?? null,
      resolvedAt: new Date(),
    })
    .where(
      and(
        eq(schema.botEvents.botId, botId),
        inArray(schema.botEvents.eventId, eventIds),
        isNull(schema.botEvents.outcome),
      ),
    )
    .returning({ id: schema.botEvents.id });
  return { updated: rows.length };
}

/**
 * Per-trigger flood breaker: when the bot's breaker rate is exceeded, the
 * trigger is disabled and the admission rejected. A human re-enables it.
 */
export async function checkTriggerBreaker(
  deps: Pick<AdmissionDeps, "db">,
  bot: BotRow,
  trigger: BotTriggerRow,
): Promise<{ tripped: boolean }> {
  const breaker = bot.breaker;
  if (!breaker) return { tripped: false };
  const since = new Date(Date.now() - breaker.windowMs);
  const [row] = await deps.db
    .select({ value: count() })
    .from(schema.botEvents)
    .where(and(eq(schema.botEvents.triggerId, trigger.id), gte(schema.botEvents.receivedAt, since)));
  if (Number(row?.value ?? 0) < breaker.fires) return { tripped: false };
  await deps.db
    .update(schema.botTriggers)
    .set({ enabled: false, disabledReason: "breaker", disabledAt: new Date() })
    .where(eq(schema.botTriggers.id, trigger.id));
  return { tripped: true };
}

/**
 * The sender's emission cap: every emit by this bot, self or addressed,
 * counted across the universe by `sender_bot_id`. The bot's breaker rate
 * applies when set; otherwise a fixed ceiling. Without publish fan-out this
 * is the whole amplification bound.
 */
export async function checkSenderRate(
  deps: Pick<AdmissionDeps, "db">,
  sender: BotRow,
): Promise<void> {
  const cap = sender.breaker ?? { fires: 60, windowMs: 60 * 60 * 1000 };
  const since = new Date(Date.now() - cap.windowMs);
  const [recent] = await deps.db
    .select({ value: count() })
    .from(schema.botEvents)
    .where(and(eq(schema.botEvents.senderBotId, sender.id), gte(schema.botEvents.receivedAt, since)));
  if (Number(recent?.value ?? 0) >= cap.fires) {
    throw new BotAdmissionRefusal(
      "rate_limited",
      `emission rate exceeded (${cap.fires} events in ${Math.round(cap.windowMs / 1000)}s); wait before emitting again`,
    );
  }
}

export interface StoreBotEventInput {
  bot: BotRow;
  /** Lightspeed universe id of the receiving bot. */
  universeId: string;
  eventId: string;
  document: BotEventDocumentV1;
  /** Salient payload projection rendered instead of `document.data`. */
  promptData?: unknown;
  triggerId?: string;
  session?: BotEventSession;
  coalesce?: BotCoalesceParamsV1;
  whenBusy?: BotWhenBusyV1;
  senderBotId?: string;
  hops?: number;
  replyTo?: { botId: string; session?: BotEventSession };
  inReplyTo?: { bot: string; seq: number };
  /** Prepared attachments appended to the run input after the rendering. */
  media?: BotEventMediaV1[];
  /** CAS ref of receiver-bound tool declarations for the routed session. */
  tools?: string;
  /** Private receipt route of the admitting source (`started` / `finished`). */
  notify?: BotEventNotifyV1;
  /** Skip the controller signal (archived events: filtered at admission). */
  deliver?: boolean;
  /** Reuse an already-stored document ref (replays). */
  ref?: string;
}

/**
 * Store, then wake: the document goes to CAS, the envelope row into the
 * authoritative store, and only then is the controller notified. A duplicate
 * admission wakes the controller again on purpose — the row may exist because
 * an earlier wake failed after the insert — and the controller dedupes by
 * event id.
 */
export async function storeBotEvent(
  deps: AdmissionDeps,
  input: StoreBotEventInput,
): Promise<{ event: BotEvent; duplicate: boolean }> {
  // The resurrection guard: waking is a signal-with-start, so a closed bot
  // must be refused on its row before anything is stored — otherwise a late
  // event would start a fresh controller for a bot that no longer exists.
  if (input.bot.closedAt !== null) {
    throw new BotAdmissionRefusal(
      "bot_closed",
      `${input.bot.name} was closed and no longer accepts events`,
    );
  }
  const seq = await allocateBotEventSeq(deps.db, input.bot.id);
  const prompt = renderAdmittedEvent(seq, input.document, input.promptData);
  const promptBlob = { bytesBase64: Buffer.from(prompt, "utf8").toString("base64") };
  let ref = input.ref;
  let promptRef: string | undefined;
  if (ref === undefined) {
    const stored = await deps.engine.call("blobs/put", {
      blobs: [
        { bytesBase64: Buffer.from(JSON.stringify(input.document), "utf8").toString("base64") },
        promptBlob,
      ],
    });
    ref = stored.result.blobs?.[0]?.blobRef;
    promptRef = stored.result.blobs?.[1]?.blobRef;
  } else {
    const stored = await deps.engine.call("blobs/put", { blobs: [promptBlob] });
    promptRef = stored.result.blobs?.[0]?.blobRef;
  }
  if (!ref || !promptRef) throw new Error("event document storage returned no ref");

  const inserted = await deps.db
    .insert(schema.botEvents)
    .values({
      botId: input.bot.id,
      eventId: input.eventId,
      seq,
      triggerId: input.triggerId ?? null,
      kind: input.document.kind,
      source: input.document.source,
      occurredAt: new Date(input.document.occurredAt),
      ref,
      promptRef,
      session: input.session ?? null,
      senderBotId: input.senderBotId ?? null,
      hops: input.hops ?? 0,
      replyTo: input.replyTo ?? null,
      inReplyTo: input.inReplyTo ?? null,
      media: input.media ?? null,
      tools: input.tools ?? null,
      notify: input.notify === undefined ? null : persistBotEventNotify(input.notify),
      // Archived rows are for the record (a chat send, a replay's original):
      // resolved at birth, never delivered.
      outcome: input.deliver === false ? "archived" : null,
      resolvedAt: input.deliver === false ? new Date() : null,
    })
    .onConflictDoNothing()
    .returning();
  const duplicate = inserted.length === 0;
  let eventSeq: number | null = seq;
  if (duplicate) {
    // Keep #N stable: a re-admitted event reuses the stored row's identity
    // (the freshly allocated seq is wasted, which only leaves a gap).
    const [stored] = await deps.db
      .select()
      .from(schema.botEvents)
      .where(
        and(eq(schema.botEvents.botId, input.bot.id), eq(schema.botEvents.eventId, input.eventId)),
      )
      .limit(1);
    if (stored) {
      ref = stored.ref;
      promptRef = stored.promptRef ?? undefined;
      eventSeq = stored.seq;
    }
  }

  const event: BotEvent = {
    version: 1,
    id: input.eventId,
    ref,
    ...(eventSeq === null ? {} : { seq: eventSeq }),
    ...(promptRef === undefined ? {} : { promptRef }),
    ...(input.session === undefined ? {} : { session: input.session }),
    ...(input.coalesce === undefined ? {} : { coalesce: input.coalesce }),
    ...(input.whenBusy === undefined ? {} : { deliver: { whenBusy: input.whenBusy } }),
    ...(input.hops === undefined || input.hops === 0 ? {} : { hops: input.hops }),
    ...(input.replyTo === undefined ? {} : { reply: true }),
    ...(input.media === undefined || input.media.length === 0 ? {} : { media: input.media }),
    ...(input.tools === undefined ? {} : { tools: input.tools }),
    ...(input.notify === undefined ? {} : { notify: true }),
  };
  if (input.deliver !== false) {
    await wakeBotController({
      db: deps.db,
      temporal: deps.temporal,
      start: botStartFor(input.bot, input.universeId),
      event,
      stored: !duplicate,
    });
  }
  return { event, duplicate };
}

export interface AdmitTriggerEventInput {
  bot: BotRow;
  trigger: BotTriggerRow;
  /** Lightspeed universe id of the receiving bot. */
  universeId: string;
  eventId: string;
  document: BotEventDocumentV1;
  promptData?: unknown;
  senderBotId?: string;
  hops?: number;
  replyTo?: { botId: string; session?: BotEventSession };
  media?: BotEventMediaV1[];
  tools?: string;
  notify?: BotEventNotifyV1;
}

export type AdmitTriggerEventResult =
  | { filtered: false; event: BotEvent; duplicate: boolean }
  /** The filter did not match (or threw, fail-closed): nothing was stored. */
  | { filtered: true; error?: string };

/**
 * The trigger pipeline every receiver-side knob runs through, in order:
 * filter (refuse on miss — a miss is never stored, so a strict filter on a
 * firehose costs nothing), route, coalesce, delivery policy, then
 * store-then-wake. The caller has already checked the trigger is enabled
 * and the breaker has not tripped.
 */
export async function admitTriggerEvent(
  deps: AdmissionDeps,
  input: AdmitTriggerEventInput,
): Promise<AdmitTriggerEventResult> {
  const { bot, trigger, document } = input;
  const context: FilterContext = {
    event: {
      id: input.eventId,
      kind: document.kind,
      source: document.source,
      occurredAt: document.occurredAt,
      ...(document.sender === undefined ? {} : { sender: document.sender.bot }),
    },
    data: document.data,
    headers: document.headers ?? {},
  };
  const base = {
    bot,
    universeId: input.universeId,
    eventId: input.eventId,
    document,
    ...(input.promptData === undefined ? {} : { promptData: input.promptData }),
    triggerId: trigger.id,
    ...(input.senderBotId === undefined ? {} : { senderBotId: input.senderBotId }),
    ...(input.hops === undefined ? {} : { hops: input.hops }),
    ...(input.replyTo === undefined ? {} : { replyTo: input.replyTo }),
    ...(input.media === undefined ? {} : { media: input.media }),
    ...(input.tools === undefined ? {} : { tools: input.tools }),
    ...(input.notify === undefined ? {} : { notify: input.notify }),
  };

  if (trigger.filter !== null) {
    const filtered = evaluateFilter(trigger.filter, context);
    if (!filtered.matched) {
      // A filter that throws is a configuration problem, not an event: it is
      // surfaced on the trigger and cleared by the next match.
      if (filtered.error !== undefined) {
        await deps.db
          .update(schema.botTriggers)
          .set({ lastFilterError: filtered.error, lastFilterErrorAt: new Date() })
          .where(eq(schema.botTriggers.id, trigger.id));
        return { filtered: true, error: filtered.error };
      }
      return { filtered: true };
    }
    if (trigger.lastFilterError !== null) {
      await deps.db
        .update(schema.botTriggers)
        .set({ lastFilterError: null, lastFilterErrorAt: null })
        .where(eq(schema.botTriggers.id, trigger.id));
    }
  }

  const preset =
    trigger.kind === "webhook"
      ? ((trigger.spec as { preset?: "github" | null }).preset ?? null)
      : trigger.kind === "chat"
        ? "chat"
        : null;
  const routed = computeRouteSession(
    bot.name,
    trigger.route,
    preset,
    { eventId: input.eventId, ...(document.data === undefined ? {} : { data: document.data }) },
    context,
  );
  // Per-trigger retention rides on the routed target: null on the row
  // inherits the bot's setting, 0 keeps the session open indefinitely.
  const session =
    routed.session === undefined || trigger.sessionTtlMs === null
      ? routed.session
      : { ...routed.session, ttlMs: trigger.sessionTtlMs === 0 ? null : trigger.sessionTtlMs };
  const { event, duplicate } = await storeBotEvent(deps, {
    ...base,
    ...(session === undefined ? {} : { session }),
    ...(trigger.coalesce === null
      ? {}
      : {
          coalesce: {
            key: `${trigger.id}|${routed.session?.sessionId ?? "main"}`,
            ...trigger.coalesce,
          },
        }),
    ...(trigger.deliver === null ? {} : { whenBusy: trigger.deliver.whenBusy }),
  });
  return { filtered: false, event, duplicate };
}

/** What the directory knows about one neighbour. */
export interface DirectoryBotRow {
  name: string;
  enabled: boolean;
  description: string | null;
  /** The neighbour's `bot`-kind trigger, if any. */
  inbox: Pick<BotTriggerRow, "enabled" | "spec"> | null;
}

export interface DirectoryEntry {
  botId: string;
  description: string | null;
}

/** Only the neighbours whose inbox accepts `me`: bots that are not listening cost context and help nobody. */
export function directoryEntriesFor(me: string, bots: DirectoryBotRow[]): DirectoryEntry[] {
  return bots
    .filter((bot) => bot.name !== me && bot.enabled && bot.inbox !== null && bot.inbox.enabled)
    .filter((bot) => {
      const from = (bot.inbox?.spec as BotInboxTriggerSpecV1 | undefined)?.from;
      return from === undefined || from.includes(me);
    })
    .map((bot) => ({ botId: bot.name, description: bot.description }))
    .sort((a, b) => a.botId.localeCompare(b.botId));
}

export const BOT_DIRECTORY_KEY = "bot:directory";
export const BOT_DIRECTORY_TITLE = "Bot directory";

/** The catalog body: one line per bot that accepts events from the reader. */
export function renderBotDirectory(entries: DirectoryEntry[]): string {
  if (entries.length === 0) {
    return "No other bot accepts events from you right now.";
  }
  return [
    "Bots that accept events addressed by you (bot_emit with to):",
    ...entries.map(
      (entry) => `- ${entry.botId}${entry.description === null ? "" : ` — ${entry.description}`}`,
    ),
  ].join("\n");
}

export interface ReceiptInput {
  /** The answering bot's id (its `name`). */
  answering: string;
  /** The asked event's #N at the answering bot. */
  askedSeq: number;
  status: string;
  summary: string | null;
  occurredAt: string;
  hops: number;
}

/** The deterministic receipt for one asked event: the delivery's outcome, never a model-authored reply. */
export function receiptDocument(input: ReceiptInput): BotEventDocumentV1 {
  return {
    version: 1,
    kind: "bot.reply",
    source: `bot:${input.answering}`,
    occurredAt: input.occurredAt,
    summary: input.summary ?? `#${input.askedSeq} at ${input.answering} finished ${input.status}`,
    data: { status: input.status },
    sender: { bot: input.answering },
    hops: input.hops,
    inReplyTo: { bot: input.answering, seq: input.askedSeq },
  };
}
