import { relations } from "drizzle-orm";
import {
  bigint,
  boolean,
  index,
  integer,
  jsonb,
  pgTable,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";
import { universes } from "./platform.js";

const createdAt = () => timestamp("created_at", { withTimezone: true }).defaultNow().notNull();
const updatedAt = () =>
  timestamp("updated_at", { withTimezone: true })
    .defaultNow()
    .$onUpdate(() => new Date())
    .notNull();

/** Durable identity and operator-owned configuration for one bot. */
export const bots = pgTable(
  "bots",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    universeId: uuid("universe_id")
      .notNull()
      .references(() => universes.id, { onDelete: "cascade" }),
    /** Authored, immutable, universe-unique id (`botId` on the wire); every Temporal and session identity derives from it. */
    name: text("name").notNull(),
    /** Mutable label for humans; falls back to the id. Never identity. */
    displayName: text("display_name"),
    /** One line other bots read in the directory; the brief is the job description for this bot. */
    description: text("description"),
    profileId: text("profile_id").notNull(),
    /** Standing instructions appended to the profile's instructions. */
    brief: text("brief"),
    /** Budget: runs started per UTC day; null means unlimited. */
    runsPerDay: integer("runs_per_day"),
    /** Per-trigger flood breaker: auto-disable a trigger exceeding this rate. */
    breaker: jsonb("breaker").$type<{ fires: number; windowMs: number }>(),
    /** Close routed (perKey/perEvent) sessions idle longer than this; null keeps them. */
    routedSessionTtlMs: integer("routed_session_ttl_ms"),
    /** Monotonic per-bot event counter; allocated at admission, shown as #N. */
    eventSeq: bigint("event_seq", { mode: "number" }).default(0).notNull(),
    /**
     * Capability grant: whether the bot's sessions get the mutating
     * self-configuration tools (trigger put/delete, brief put). Off by
     * default — self-modification is opted into per bot, never assumed.
     */
    selfConfig: boolean("self_config").default(false).notNull(),
    /**
     * Capability grant: whether the bot's sessions get `bot_emit` — events to
     * itself or addressed to another bot's inbox. Off by default; emitting
     * bots are rate-capped to break feedback loops.
     */
    emit: boolean("emit").default(false).notNull(),
    enabled: boolean("enabled").default(true).notNull(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [uniqueIndex("bots_universe_name_idx").on(t.universeId, t.name)],
);

export type BotScheduleTriggerSpec = {
  cron?: string | null;
  at?: string | null;
  timezone: string;
  summary: string;
};
export type BotWebhookTriggerSpec = {
  token: string;
  verification:
    | { scheme: "token" }
    | { scheme: "hmac-sha256"; grantId: string; header: string; prefix?: string; audience?: string };
  preset?: "github" | null;
};
export type BotPollTriggerSpec = {
  source:
    | {
        kind: "http";
        url: string;
        method?: "GET" | "POST";
        /** Non-secret headers only; credentials are resolved through `auth`. */
        headers?: Record<string, string>;
        auth?: { grantId: string; header?: string; scheme?: string; audience?: string };
        body?: string;
      }
    | { kind: "exec"; environmentId: string; argv: string[]; cwd?: string | null; timeoutMs?: number | null };
  intervalMs: number;
  items?: string | null;
  cursor: { kind: "idSet"; id: string } | { kind: "watermark"; field: string };
};
/** Inbox: which bots may address this one; absent = any bot in the universe. */
export type BotInboxTriggerSpec = {
  from?: string[];
};
/**
 * A chat connection: a provider account, which conversations it serves, how
 * the bot activates in groups, who may talk to it, and the pairing gate.
 * Routing, coalescing, filters, and delivery policy are the trigger's
 * generic columns; the bot's profile and brief are the bot's.
 */
export type BotChatTriggerSpec = {
  channelAccountId: string;
  matchScope: "direct" | "group" | null;
  activation: {
    group?: "mention" | "always";
    triggerPrefixes?: string[];
    mentionNames?: string[];
  } | null;
  access: {
    turn?: "conversation" | "members";
    control?: "none" | "members" | "admins" | "owners";
  } | null;
  /** Null pairs implicitly (an open connection). */
  pairingCode: string | null;
  /** Lower wins among matching chat triggers on one account. */
  priority: number;
};
export type BotTriggerSpec =
  | BotScheduleTriggerSpec
  | BotWebhookTriggerSpec
  | BotPollTriggerSpec
  | BotInboxTriggerSpec
  | BotChatTriggerSpec;

/** A prepared attachment carried by an event into the run input; bytes live in CAS. */
export type BotEventMedia = {
  blobRef: string;
  kind: "image" | "audio" | "document";
  mime: string;
  name?: string | null;
};

/**
 * Delivery receipts for the admitting source: a workflow endpoint signalled
 * with `started` / `finished` and the caller's opaque token. Never on the
 * wire and never shown to the model.
 */
export type BotEventNotify = { workflowId: string; workflowKind: string; token: string };

/** Poll cursor state: Lightspeed-owned, operator-visible, resettable. */
export type BotPollCursorState = {
  ids?: string[];
  watermark?: string | number;
  consecutiveFailures: number;
  baselinedAt?: string;
  lastPolledAt?: string;
};
export type BotTriggerRoute =
  | { policy: "bot" }
  | { policy: "perKey"; key?: string | null }
  | { policy: "perEvent" };

/**
 * One configured trigger per row. Schedule triggers reconcile to a Temporal
 * Schedule that starts the fire workflow; webhook triggers are addressed by
 * per-trigger ingest URLs. The row stays authoritative and admission re-reads
 * it, so stale external state can never admit stale config.
 */
export const botTriggers = pgTable(
  "bot_triggers",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    botId: uuid("bot_id")
      .notNull()
      .references(() => bots.id, { onDelete: "cascade" }),
    name: text("name").notNull(),
    /** `bot` is the inbox for events other bots address here; at most one per bot. */
    kind: text("kind", { enum: ["schedule", "webhook", "poll", "bot", "chat"] }).notNull(),
    /** Per-kind configuration document. */
    spec: jsonb("spec").$type<BotTriggerSpec>().notNull(),
    /** CEL over {event, data, headers}; non-matching events archive instead of delivering. */
    filter: text("filter"),
    /** Session routing policy; null routes to the bot's main session. */
    route: jsonb("route").$type<BotTriggerRoute>(),
    /** Coalescing window; events sharing a route flush as one delivery. */
    coalesce: jsonb("coalesce").$type<{
      debounceMs: number;
      maxWaitMs: number;
      maxCount: number;
    }>(),
    /** Delivery policy when the target session has an active run. */
    deliver: jsonb("deliver").$type<{ whenBusy: "queue" | "steer" | "append" }>(),
    /** Poll kind only: the advancing cursor; null until the baseline poll. */
    cursor: jsonb("cursor").$type<BotPollCursorState>(),
    /**
     * Retention of the sessions this trigger routes to: null inherits the
     * bot's `routedSessionTtlMs`, 0 keeps them open indefinitely (chat).
     */
    sessionTtlMs: integer("session_ttl_ms"),
    enabled: boolean("enabled").default(true).notNull(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [uniqueIndex("bot_triggers_bot_name_idx").on(t.botId, t.name)],
);

/**
 * Authoritative event envelope store; the controller signal is a notification
 * over this table, never the system of record. Payload documents live in CAS.
 */
export const botEvents = pgTable(
  "bot_events",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    botId: uuid("bot_id")
      .notNull()
      .references(() => bots.id, { onDelete: "cascade" }),
    /** Dedupe identity: provider delivery id where known, otherwise generated. */
    eventId: text("event_id").notNull(),
    /**
     * Per-bot sequence number (#N): the only event handle shown to models and
     * humans. Null only for rows that predate sequence numbering.
     */
    seq: bigint("seq", { mode: "number" }),
    /** Originating trigger; null for direct endpoint/manual admissions. */
    triggerId: uuid("trigger_id").references(() => botTriggers.id, { onDelete: "set null" }),
    kind: text("kind").notNull(),
    source: text("source").notNull(),
    occurredAt: timestamp("occurred_at", { withTimezone: true }).notNull(),
    /** CAS blob ref of the event document. */
    ref: text("ref").notNull(),
    /** CAS blob ref of the model-facing rendering delivered to sessions. */
    promptRef: text("prompt_ref"),
    /** Routed session target recorded at admission; replay reuses it. */
    session: jsonb("session").$type<{ sessionId: string; label: string }>(),
    /** Sending bot for bot-originated events (self or addressed); null for world events. */
    senderBotId: uuid("sender_bot_id").references(() => bots.id, { onDelete: "set null" }),
    /** Federation loop bound: 0 for world events, the causing delivery's highest + 1 for bot events. */
    hops: integer("hops").default(0).notNull(),
    /**
     * Private return route of an addressed event that asked for a receipt:
     * the asking bot and its logical session (base id, never a generation).
     * Never on the wire.
     */
    replyTo: jsonb("reply_to").$type<{ botId: string; session?: { sessionId: string; label: string } }>(),
    /** Public correlation of a receipt: the asked event's #N at the answering bot. */
    inReplyTo: jsonb("in_reply_to").$type<{ bot: string; seq: number }>(),
    /** Prepared attachments appended to the run input after the rendering. */
    media: jsonb("media").$type<BotEventMedia[]>(),
    /**
     * CAS ref of receiver-bound tool declarations the routed session is
     * created with (a chat conversation's `message_*` tools). Opaque here;
     * identical for every event of one routed session by construction.
     */
    tools: text("tools"),
    /** Private delivery-receipt route of the admitting source. */
    notify: jsonb("notify").$type<BotEventNotify>(),
    receivedAt: createdAt(),
  },
  (t) => [
    uniqueIndex("bot_events_bot_event_idx").on(t.botId, t.eventId),
    uniqueIndex("bot_events_bot_seq_idx").on(t.botId, t.seq),
    index("bot_events_bot_received_idx").on(t.botId, t.receivedAt),
  ],
);

/** Event-to-decision-to-run trace written by the bot controller. */
export const botActivity = pgTable(
  "bot_activity",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    botId: uuid("bot_id")
      .notNull()
      .references(() => bots.id, { onDelete: "cascade" }),
    kind: text("kind").notNull(),
    eventId: text("event_id"),
    runId: text("run_id"),
    detail: text("detail"),
    createdAt: createdAt(),
  },
  (t) => [index("bot_activity_bot_created_idx").on(t.botId, t.createdAt)],
);

export const botsRelations = relations(bots, ({ one, many }) => ({
  universe: one(universes, {
    fields: [bots.universeId],
    references: [universes.id],
  }),
  triggers: many(botTriggers),
  events: many(botEvents),
  activity: many(botActivity),
}));

export const botTriggersRelations = relations(botTriggers, ({ one }) => ({
  bot: one(bots, {
    fields: [botTriggers.botId],
    references: [bots.id],
  }),
}));

export const botEventsRelations = relations(botEvents, ({ one }) => ({
  bot: one(bots, {
    fields: [botEvents.botId],
    references: [bots.id],
  }),
}));

export const botActivityRelations = relations(botActivity, ({ one }) => ({
  bot: one(bots, {
    fields: [botActivity.botId],
    references: [bots.id],
  }),
}));
