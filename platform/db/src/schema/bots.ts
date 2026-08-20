import { relations } from "drizzle-orm";
import {
  boolean,
  index,
  integer,
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
    name: text("name").notNull(),
    profileId: text("profile_id").notNull(),
    /** Standing instructions appended to the profile's instructions. */
    brief: text("brief"),
    /** Budget: runs started per UTC day; null means unlimited. */
    runsPerDay: integer("runs_per_day"),
    enabled: boolean("enabled").default(true).notNull(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [uniqueIndex("bots_universe_name_idx").on(t.universeId, t.name)],
);

/**
 * One configured trigger per row. Schedule triggers reconcile to a Temporal
 * Schedule that starts the fire workflow; the row stays authoritative and the
 * fire activity re-reads it, so a stale schedule can never admit stale config.
 */
export const botTriggers = pgTable(
  "bot_triggers",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    botId: uuid("bot_id")
      .notNull()
      .references(() => bots.id, { onDelete: "cascade" }),
    name: text("name").notNull(),
    kind: text("kind", { enum: ["schedule"] }).notNull(),
    cron: text("cron").notNull(),
    timezone: text("timezone").default("UTC").notNull(),
    /** What the fired event asks the session to do. */
    summary: text("summary").notNull(),
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
    kind: text("kind").notNull(),
    source: text("source").notNull(),
    occurredAt: timestamp("occurred_at", { withTimezone: true }).notNull(),
    /** CAS blob ref of the event document. */
    ref: text("ref").notNull(),
    receivedAt: createdAt(),
  },
  (t) => [
    uniqueIndex("bot_events_bot_event_idx").on(t.botId, t.eventId),
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
