import { relations } from "drizzle-orm";
import {
  boolean,
  foreignKey,
  index,
  integer,
  jsonb,
  pgTable,
  text,
  timestamp,
  unique,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";
import { user } from "./auth.js";
import { universes } from "./platform.js";

const createdAt = () => timestamp("created_at", { withTimezone: true }).defaultNow().notNull();
const updatedAt = () =>
  timestamp("updated_at", { withTimezone: true })
    .defaultNow()
    .$onUpdate(() => new Date())
    .notNull();

export type ChannelAccountSettings = {
  printQr?: boolean;
};

export type ChannelBindingActivationSettings = {
  group?: "mention" | "always" | "silent";
  triggerPrefixes?: string[];
  mentionNames?: string[];
  roomContextLimit?: number;
  batching?: {
    debounceMs?: number;
    maxWaitMs?: number;
    maxMessages?: number;
  };
};

export type ChannelBindingAccessSettings = {
  turn?: "conversation" | "members";
  control?: "none" | "members" | "admins" | "owners";
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

/// Routing rules: which conversations reach which universe under which
/// profile, activation policy, and sender authorization policy.
export const channelBindings = pgTable(
  "channel_bindings",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    universeId: uuid("universe_id")
      .notNull()
      .references(() => universes.id, { onDelete: "cascade" }),
    channelAccountId: uuid("channel_account_id")
      .notNull()
      .references(() => channelAccounts.id, { onDelete: "cascade" }),
    /// Stable rule name used by Channels pairing; unique per universe.
    name: text("name").notNull(),
    /// Channel scope vocabulary: `direct` or `group`.
    matchScope: text("match_scope", { enum: ["direct", "group"] }),
    /// Lightspeed profile id applied to sessions bound by this rule.
    profileId: text("profile_id"),
    sessionKey: text("session_key").notNull(),
    /// Workflow-owned activation and durable batching policy.
    activation: jsonb("activation").$type<ChannelBindingActivationSettings>(),
    /// Sender and control-command authorization policy.
    access: jsonb("access").$type<ChannelBindingAccessSettings>(),
    pairingCode: text("pairing_code"),
    /// Lower number wins when multiple rules match a conversation.
    priority: integer("priority").default(100).notNull(),
    enabled: boolean("enabled").default(true).notNull(),
    createdAt: createdAt(),
    updatedAt: updatedAt(),
  },
  (t) => [
    index("channel_bindings_universe_idx").on(t.universeId),
    index("channel_bindings_channel_account_idx").on(t.channelAccountId),
    uniqueIndex("channel_bindings_universe_name_idx").on(t.universeId, t.name),
    unique("channel_bindings_id_account_unique").on(t.id, t.channelAccountId),
  ],
);

/// A route authorized against a binding rule. The opaque key is derived from
/// provider/account/chat without retaining message data.
export const channelPairings = pgTable(
  "channel_pairings",
  {
    key: text("key").primaryKey(),
    bindingId: uuid("binding_id").notNull(),
    channelAccountId: uuid("channel_account_id")
      .notNull()
      .references(() => channelAccounts.id, { onDelete: "cascade" }),
    chatId: text("chat_id").notNull(),
    pairedAt: timestamp("paired_at", { withTimezone: true }).defaultNow().notNull(),
    updatedAt: updatedAt(),
  },
  (t) => [
    index("channel_pairings_account_chat_idx").on(t.channelAccountId, t.chatId),
    foreignKey({
      name: "channel_pairings_binding_account_fk",
      columns: [t.bindingId, t.channelAccountId],
      foreignColumns: [channelBindings.id, channelBindings.channelAccountId],
    }).onDelete("cascade"),
  ],
);

export const channelBindingsRelations = relations(channelBindings, ({ one }) => ({
  universe: one(universes, {
    fields: [channelBindings.universeId],
    references: [universes.id],
  }),
  channelAccount: one(channelAccounts, {
    fields: [channelBindings.channelAccountId],
    references: [channelAccounts.id],
  }),
}));

export const channelPairingsRelations = relations(channelPairings, ({ one }) => ({
  binding: one(channelBindings, {
    fields: [channelPairings.bindingId],
    references: [channelBindings.id],
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
