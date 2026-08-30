-- P142 channels: chat provider accounts served by the connector host and
-- the conversation pairings that authorize a chat against a bot's chat
-- trigger.
--
-- Design notes:
-- - Accounts are universe resources with authored ids; the credential is a
--   grant reference in the document, never a token. One token serves one
--   universe; the connector host discovers accounts across universes.
-- - Channel pairings cascade from both the chat trigger (`bot_triggers`,
--   `008_bots.sql`) and the account; a re-pair replaces the row for the
--   same `pairing_key`.

-- ═══════════════════════════════════════════════════════════════════════════
-- channel_accounts
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS channel_accounts (
    -- ── Identity ───────────────────────────────────────────────────────────
    universe_id uuid NOT NULL
        REFERENCES universes (universe_id) ON DELETE CASCADE,
    -- Authored account id, unique per universe; chat triggers point at it
    -- and the connector queue name derives from it.
    account_id text NOT NULL,
    -- Copy of the document's provider for indexed listing: telegram |
    -- whatsapp.
    provider text NOT NULL,
    -- Provider-native account identity (Telegram bot username or id,
    -- WhatsApp phone number); unique per universe and provider.
    provider_account_id text NOT NULL,

    -- ── The operator's document ────────────────────────────────────────────
    -- Bumped by every put.
    revision bigint NOT NULL,
    -- ChannelAccountDocument: provider, providerAccountId, displayName,
    -- credentialGrantId, settings, enabled. Never secret material.
    document_json jsonb NOT NULL,

    -- ── Runtime-owned ──────────────────────────────────────────────────────
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, account_id),
    CONSTRAINT channel_accounts_provider_account_unique
        UNIQUE (universe_id, provider, provider_account_id),

    CONSTRAINT channel_accounts_account_id_format
        CHECK (account_id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
    CONSTRAINT channel_accounts_provider_known
        CHECK (provider IN ('telegram', 'whatsapp')),
    CONSTRAINT channel_accounts_provider_account_id_not_empty
        CHECK (provider_account_id <> ''),
    CONSTRAINT channel_accounts_revision_positive
        CHECK (revision > 0),
    CONSTRAINT channel_accounts_document_object
        CHECK (jsonb_typeof(document_json) = 'object'),
    CONSTRAINT channel_accounts_created_nonnegative
        CHECK (created_at_ms >= 0),
    CONSTRAINT channel_accounts_updated_after_created
        CHECK (updated_at_ms >= created_at_ms)
);

COMMENT ON TABLE channel_accounts IS
    'Chat provider accounts served by the connector host; routing identity and operational configuration, never secret material.';
COMMENT ON COLUMN channel_accounts.account_id IS
    'Authored channel account id, unique per universe; chat triggers point at it.';
COMMENT ON COLUMN channel_accounts.provider IS
    'Chat provider copied from the document for indexed listing; telegram|whatsapp.';
COMMENT ON COLUMN channel_accounts.provider_account_id IS
    'Provider-native account identity (Telegram bot username or id, WhatsApp phone number); unique per universe and provider.';
COMMENT ON COLUMN channel_accounts.revision IS
    'Document revision, bumped by every put.';
COMMENT ON COLUMN channel_accounts.document_json IS
    'Serialized ChannelAccountDocument; the credential is a grant reference, never a token.';
COMMENT ON COLUMN channel_accounts.created_at_ms IS
    'Creation time in Unix milliseconds.';
COMMENT ON COLUMN channel_accounts.updated_at_ms IS
    'Last document write in Unix milliseconds.';

-- ═══════════════════════════════════════════════════════════════════════════
-- channel_pairings
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS channel_pairings (
    -- ── Identity ───────────────────────────────────────────────────────────
    universe_id uuid NOT NULL,
    -- Opaque key derived from account and chat; never message data.
    pairing_key text NOT NULL,

    -- ── What paired with what ──────────────────────────────────────────────
    -- The chat trigger the conversation paired with; a re-pair moves the
    -- chat to another trigger.
    bot_id text NOT NULL,
    trigger_id text NOT NULL,
    -- Channel account the conversation arrived through.
    account_id text NOT NULL,
    -- Provider chat identifier of the conversation.
    chat_id text NOT NULL,
    paired_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, pairing_key),
    CONSTRAINT channel_pairings_trigger_fk FOREIGN KEY (universe_id, bot_id, trigger_id)
        REFERENCES bot_triggers (universe_id, bot_id, trigger_id) ON DELETE CASCADE,
    CONSTRAINT channel_pairings_account_fk FOREIGN KEY (universe_id, account_id)
        REFERENCES channel_accounts (universe_id, account_id) ON DELETE CASCADE,

    CONSTRAINT channel_pairings_pairing_key_not_empty
        CHECK (pairing_key <> ''),
    CONSTRAINT channel_pairings_chat_id_not_empty
        CHECK (chat_id <> ''),
    CONSTRAINT channel_pairings_paired_nonnegative
        CHECK (paired_at_ms >= 0)
);

-- Inbound lookup: is this chat paired on this account?
CREATE INDEX IF NOT EXISTS channel_pairings_chat_idx
    ON channel_pairings (universe_id, account_id, chat_id);

-- The trigger's pairing list.
CREATE INDEX IF NOT EXISTS channel_pairings_trigger_idx
    ON channel_pairings (universe_id, bot_id, trigger_id);

COMMENT ON TABLE channel_pairings IS
    'Conversations authorized against a chat trigger by pairing code (or implicitly for open triggers); cascades from the trigger and the account.';
COMMENT ON COLUMN channel_pairings.pairing_key IS
    'Opaque key derived from account and chat; never message data.';
COMMENT ON COLUMN channel_pairings.bot_id IS
    'Bot whose chat trigger serves the conversation.';
COMMENT ON COLUMN channel_pairings.trigger_id IS
    'The chat trigger the conversation paired with; a re-pair moves the chat to another trigger.';
COMMENT ON COLUMN channel_pairings.account_id IS
    'Channel account the conversation arrived through.';
COMMENT ON COLUMN channel_pairings.chat_id IS
    'Provider chat identifier of the conversation.';
COMMENT ON COLUMN channel_pairings.paired_at_ms IS
    'When the conversation paired, in Unix milliseconds.';
