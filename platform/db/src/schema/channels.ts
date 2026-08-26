import { relations } from "drizzle-orm";
import {
  boolean,
  index,
  jsonb,
  pgTable,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";
import { user } from "./auth.js";
import { botTriggers } from "./bots.js";

const createdAt = () => timestamp("created_at", { withTimezone: true }).defaultNow().notNull();
const updatedAt = () =>
  timestamp("updated_at", { withTimezone: true })
    .defaultNow()
    .$onUpdate(() => new Date())
    .notNull();

export type ChannelAccountSettings = {
  printQr?: boolean;
};

/// Provider accounts are control-plane resources. Secret material remains in
/// the referenced environment/secret store and WhatsApp auth volume; this row
/// contains only routing identity and operational configuration.
export const channelAccounts = pgTable(
  "channel_accounts",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    provider: text("provider", { enum: ["telegram", "whatsapp"] }).notNull(),
    accountId: text("account_id").notNull(),
    displayName: text("display_name").notNull(),
    credentialRef: text("credential_ref"),
    stateRef: text("state_ref"),
    settings: jsonb("settings").$type<ChannelAccountSettings>().default({}).notNull(),
    enabled: boolean("enabled").default(true).notNull(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [
    uniqueIndex("channel_accounts_provider_account_idx").on(t.provider, t.accountId),
  ],
);

/// A human's handle on a channel, linked to their platform user. Written by
/// pairing flows; lets one person span Telegram + WhatsApp as one identity.
export const channelIdentities = pgTable(
  "channel_identities",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: text("user_id")
      .notNull()
      .references(() => user.id, { onDelete: "cascade" }),
    channel: text("channel", { enum: ["telegram", "whatsapp"] }).notNull(),
    /// Channel-native sender id: Telegram numeric id, WhatsApp JID.
    handle: text("handle").notNull(),
    displayName: text("display_name"),
    createdAt: createdAt(),
  },
  (t) => [uniqueIndex("channel_identities_channel_handle_idx").on(t.channel, t.handle)],
);

/// A conversation authorized against a `chat` bot trigger by its pairing
/// code. The opaque key is derived from provider/account/chat without
/// retaining message data. Chat routing itself lives on `bot_triggers`
/// (kind `chat`): a chat connection is a bot trigger, never its own record.
export const channelPairings = pgTable(
  "channel_pairings",
  {
    key: text("key").primaryKey(),
    triggerId: uuid("trigger_id")
      .notNull()
      .references(() => botTriggers.id, { onDelete: "cascade" }),
    channelAccountId: uuid("channel_account_id")
      .notNull()
      .references(() => channelAccounts.id, { onDelete: "cascade" }),
    chatId: text("chat_id").notNull(),
    pairedAt: timestamp("paired_at", { withTimezone: true }).defaultNow().notNull(),
    updatedAt: updatedAt(),
  },
  (t) => [
    index("channel_pairings_account_chat_idx").on(t.channelAccountId, t.chatId),
    index("channel_pairings_trigger_idx").on(t.triggerId),
  ],
);

export const channelPairingsRelations = relations(channelPairings, ({ one }) => ({
  trigger: one(botTriggers, {
    fields: [channelPairings.triggerId],
    references: [botTriggers.id],
  }),
  channelAccount: one(channelAccounts, {
    fields: [channelPairings.channelAccountId],
    references: [channelAccounts.id],
  }),
}));

export const channelIdentitiesRelations = relations(channelIdentities, ({ one }) => ({
  user: one(user, {
    fields: [channelIdentities.userId],
    references: [user.id],
  }),
}));
