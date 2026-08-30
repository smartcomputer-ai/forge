-- P142 bots: the bot registry, its triggers, and the per-bot event log.
--
-- Design notes:
-- - Bot decisions live in the controller's Temporal history; these tables
--   are the read model. `bot_events` is the bot's numbered log of what
--   arrived and what it sent, each row with a write-once `outcome`; trigger
--   incidents (`disabled_reason`, `last_filter_error`) are trigger state.
--   A filter miss is never stored.
-- - A bot is addressed by its authored, immutable `bot_id`; there is no
--   surrogate key. `display_name` lives in the document and may change.
-- - Event rows keep `trigger_id` without a foreign key: a trigger may be
--   deleted while its history stays readable.
-- - Trigger secrets (webhook URL token, pairing code) sit beside the
--   document in `secrets_json` and are shown only to managing principals.
-- - Chat provider accounts and conversation pairings live in
--   `009_channels.sql`.

CREATE TABLE IF NOT EXISTS bots (
    universe_id uuid NOT NULL
        REFERENCES universes (universe_id) ON DELETE CASCADE,
    bot_id text NOT NULL,
    revision bigint NOT NULL,
    document_json jsonb NOT NULL,
    event_seq bigint NOT NULL DEFAULT 0,
    closed_at_ms bigint,
    closed_sessions_json jsonb NOT NULL DEFAULT '[]',
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, bot_id),

    CONSTRAINT bots_bot_id_format
        CHECK (bot_id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
    CONSTRAINT bots_revision_positive
        CHECK (revision > 0),
    CONSTRAINT bots_document_object
        CHECK (jsonb_typeof(document_json) = 'object'),
    CONSTRAINT bots_event_seq_nonnegative
        CHECK (event_seq >= 0),
    CONSTRAINT bots_closed_at_ms_nonnegative
        CHECK (closed_at_ms IS NULL OR closed_at_ms >= 0),
    CONSTRAINT bots_closed_sessions_array
        CHECK (jsonb_typeof(closed_sessions_json) = 'array'),
    CONSTRAINT bots_created_nonnegative
        CHECK (created_at_ms >= 0),
    CONSTRAINT bots_updated_after_created
        CHECK (updated_at_ms >= created_at_ms)
);

COMMENT ON TABLE bots IS
    'Universe-scoped bot registry: the mutable document, the #N event counter, and the terminal close marker.';
COMMENT ON COLUMN bots.bot_id IS
    'Authored, immutable bot id; the name models and Temporal identities address the bot by.';
COMMENT ON COLUMN bots.revision IS
    'Document revision, bumped by every put, close, and runtime disable; checked by put-with-expected-revision.';
COMMENT ON COLUMN bots.document_json IS
    'Serialized BotDocument (display name, description, profile, brief, budgets, grants, enabled).';
COMMENT ON COLUMN bots.event_seq IS
    'Highest #N allocated so far; incremented atomically at admission.';
COMMENT ON COLUMN bots.closed_at_ms IS
    'Terminal close marker, set once by bots/close; a closed bot refuses events and cannot be re-enabled.';
COMMENT ON COLUMN bots.closed_sessions_json IS
    'Session ids the controller force-closed on the way out, recorded so bots/delete can erase them.';
COMMENT ON COLUMN bots.created_at_ms IS
    'Creation time in Unix milliseconds.';
COMMENT ON COLUMN bots.updated_at_ms IS
    'Last document write in Unix milliseconds.';

CREATE TABLE IF NOT EXISTS bot_triggers (
    universe_id uuid NOT NULL,
    bot_id text NOT NULL,
    trigger_id text NOT NULL,
    kind text NOT NULL,
    revision bigint NOT NULL,
    document_json jsonb NOT NULL,
    secrets_json jsonb NOT NULL DEFAULT '{}',
    disabled_reason text,
    disabled_at_ms bigint,
    last_filter_error text,
    last_filter_error_at_ms bigint,
    cursor_json jsonb,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, bot_id, trigger_id),
    CONSTRAINT bot_triggers_bot_fk FOREIGN KEY (universe_id, bot_id)
        REFERENCES bots (universe_id, bot_id) ON DELETE CASCADE,

    CONSTRAINT bot_triggers_trigger_id_format
        CHECK (trigger_id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
    CONSTRAINT bot_triggers_kind_known
        CHECK (kind IN ('schedule', 'webhook', 'poll', 'bot', 'chat')),
    CONSTRAINT bot_triggers_kind_matches_document
        CHECK (document_json->>'kind' = kind),
    CONSTRAINT bot_triggers_revision_positive
        CHECK (revision > 0),
    CONSTRAINT bot_triggers_document_object
        CHECK (jsonb_typeof(document_json) = 'object'),
    CONSTRAINT bot_triggers_secrets_object
        CHECK (jsonb_typeof(secrets_json) = 'object'),
    CONSTRAINT bot_triggers_disabled_reason_known
        CHECK (
            disabled_reason IS NULL
            OR disabled_reason IN ('breaker', 'poll_failed', 'one_shot', 'operator', 'bot_closed')
        ),
    CONSTRAINT bot_triggers_disabled_pair
        CHECK ((disabled_reason IS NULL) = (disabled_at_ms IS NULL)),
    CONSTRAINT bot_triggers_disabled_at_ms_nonnegative
        CHECK (disabled_at_ms IS NULL OR disabled_at_ms >= 0),
    CONSTRAINT bot_triggers_filter_error_pair
        CHECK ((last_filter_error IS NULL) = (last_filter_error_at_ms IS NULL)),
    CONSTRAINT bot_triggers_filter_error_at_ms_nonnegative
        CHECK (last_filter_error_at_ms IS NULL OR last_filter_error_at_ms >= 0),
    CONSTRAINT bot_triggers_cursor_object
        CHECK (cursor_json IS NULL OR jsonb_typeof(cursor_json) = 'object'),
    CONSTRAINT bot_triggers_created_nonnegative
        CHECK (created_at_ms >= 0),
    CONSTRAINT bot_triggers_updated_after_created
        CHECK (updated_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS bot_triggers_kind_idx
    ON bot_triggers (universe_id, kind, bot_id, trigger_id);

-- At most one inbox (`kind = 'bot'`) per bot: the inbox owns the bot's
-- filter, route, coalesce, and delivery policy for addressed events.
CREATE UNIQUE INDEX IF NOT EXISTS bot_triggers_inbox_unique_idx
    ON bot_triggers (universe_id, bot_id)
    WHERE kind = 'bot';

COMMENT ON TABLE bot_triggers IS
    'Triggers of a bot (schedule, webhook, poll, inbox, chat): the revisioned document, server-minted secrets, and runtime incidents.';
COMMENT ON COLUMN bot_triggers.trigger_id IS
    'Authored trigger id, unique per bot.';
COMMENT ON COLUMN bot_triggers.kind IS
    'Trigger kind copied from the document spec for indexed listing; schedule|webhook|poll|bot|chat.';
COMMENT ON COLUMN bot_triggers.revision IS
    'Document revision, bumped by every put and runtime disable.';
COMMENT ON COLUMN bot_triggers.document_json IS
    'Serialized BotTriggerDocument: the kind-specific spec plus filter, route, coalesce, deliver, sessionTtlMs, enabled.';
COMMENT ON COLUMN bot_triggers.secrets_json IS
    'Server-minted trigger secrets (webhookToken, pairingCode); shown only to managing principals.';
COMMENT ON COLUMN bot_triggers.disabled_reason IS
    'Why the runtime disabled the trigger (breaker|poll_failed|one_shot|operator|bot_closed); cleared when a put enables it again.';
COMMENT ON COLUMN bot_triggers.disabled_at_ms IS
    'When the runtime disabled the trigger, in Unix milliseconds.';
COMMENT ON COLUMN bot_triggers.last_filter_error IS
    'Last runtime failure of the CEL filter (the event was refused); cleared by the next match.';
COMMENT ON COLUMN bot_triggers.last_filter_error_at_ms IS
    'When the last filter failure happened, in Unix milliseconds.';
COMMENT ON COLUMN bot_triggers.cursor_json IS
    'Poll triggers: the advancing PollCursorState; absent until the baseline poll, reset by a spec edit.';
COMMENT ON COLUMN bot_triggers.created_at_ms IS
    'Creation time in Unix milliseconds.';
COMMENT ON COLUMN bot_triggers.updated_at_ms IS
    'Last document write in Unix milliseconds.';

CREATE TABLE IF NOT EXISTS bot_events (
    universe_id uuid NOT NULL,
    bot_id text NOT NULL,
    event_id text NOT NULL,
    seq bigint NOT NULL,
    trigger_id text,
    kind text NOT NULL,
    source text NOT NULL,
    summary text NOT NULL,
    occurred_at_ms bigint NOT NULL,
    received_at_ms bigint NOT NULL,
    document_ref text NOT NULL,
    prompt_ref text,
    session_json jsonb,
    sender_bot_id text,
    hops integer NOT NULL DEFAULT 0,
    reply_to_json jsonb,
    in_reply_to_json jsonb,
    media_json jsonb NOT NULL DEFAULT '[]',
    tools_ref text,
    notify_json jsonb,
    outcome text,
    outcome_detail text,
    delivery_id text,
    run_id text,
    resolved_at_ms bigint,

    PRIMARY KEY (universe_id, bot_id, event_id),
    CONSTRAINT bot_events_bot_fk FOREIGN KEY (universe_id, bot_id)
        REFERENCES bots (universe_id, bot_id) ON DELETE CASCADE,
    CONSTRAINT bot_events_seq_unique UNIQUE (universe_id, bot_id, seq),

    CONSTRAINT bot_events_event_id_not_empty
        CHECK (event_id <> ''),
    CONSTRAINT bot_events_seq_nonnegative
        CHECK (seq >= 0),
    CONSTRAINT bot_events_trigger_id_format
        CHECK (trigger_id IS NULL OR trigger_id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
    CONSTRAINT bot_events_kind_not_empty
        CHECK (kind <> ''),
    CONSTRAINT bot_events_occurred_nonnegative
        CHECK (occurred_at_ms >= 0),
    CONSTRAINT bot_events_received_nonnegative
        CHECK (received_at_ms >= 0),
    CONSTRAINT bot_events_document_ref_format
        CHECK (document_ref ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT bot_events_prompt_ref_format
        CHECK (prompt_ref IS NULL OR prompt_ref ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT bot_events_session_object
        CHECK (session_json IS NULL OR jsonb_typeof(session_json) = 'object'),
    CONSTRAINT bot_events_sender_bot_id_format
        CHECK (sender_bot_id IS NULL OR sender_bot_id ~ '^[a-z0-9][a-z0-9-]{0,63}$'),
    CONSTRAINT bot_events_hops_nonnegative
        CHECK (hops >= 0),
    CONSTRAINT bot_events_reply_to_object
        CHECK (reply_to_json IS NULL OR jsonb_typeof(reply_to_json) = 'object'),
    CONSTRAINT bot_events_in_reply_to_object
        CHECK (in_reply_to_json IS NULL OR jsonb_typeof(in_reply_to_json) = 'object'),
    CONSTRAINT bot_events_media_array
        CHECK (jsonb_typeof(media_json) = 'array'),
    CONSTRAINT bot_events_tools_ref_format
        CHECK (tools_ref IS NULL OR tools_ref ~ '^sha256:[0-9a-f]{64}$'),
    CONSTRAINT bot_events_notify_object
        CHECK (notify_json IS NULL OR jsonb_typeof(notify_json) = 'object'),
    CONSTRAINT bot_events_outcome_known
        CHECK (
            outcome IS NULL
            OR outcome IN (
                'handled', 'deferred', 'ignored', 'blocked', 'unresolved',
                'run_failed', 'steered', 'appended', 'archived'
            )
        ),
    CONSTRAINT bot_events_resolved_nonnegative
        CHECK (resolved_at_ms IS NULL OR resolved_at_ms >= 0)
);

CREATE INDEX IF NOT EXISTS bot_events_log_idx
    ON bot_events (universe_id, bot_id, received_at_ms DESC, seq DESC);

CREATE INDEX IF NOT EXISTS bot_events_trigger_rate_idx
    ON bot_events (universe_id, bot_id, trigger_id, received_at_ms);

CREATE INDEX IF NOT EXISTS bot_events_sender_rate_idx
    ON bot_events (universe_id, sender_bot_id, received_at_ms)
    WHERE sender_bot_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS bot_events_pending_idx
    ON bot_events (universe_id, bot_id)
    WHERE outcome IS NULL;

COMMENT ON TABLE bot_events IS
    'Per-bot numbered event log: what arrived and what the bot sent, each with a write-once delivery outcome. Filter misses are never stored.';
COMMENT ON COLUMN bot_events.event_id IS
    'Dedupe identity: the provider delivery id where known, otherwise derived at admission.';
COMMENT ON COLUMN bot_events.seq IS
    'Per-bot #N, allocated from bots.event_seq; the only event handle shown to models and humans.';
COMMENT ON COLUMN bot_events.trigger_id IS
    'Admitting trigger; no foreign key so history outlives the trigger. NULL for events with no trigger (an operator admit).';
COMMENT ON COLUMN bot_events.kind IS
    'Event kind as authored by the source (e.g. github.push, schedule.fire, chat.message).';
COMMENT ON COLUMN bot_events.source IS
    'Where the event came from (trigger kind, connector, or sending bot), for display.';
COMMENT ON COLUMN bot_events.summary IS
    'One-line human summary rendered in the log and the roster.';
COMMENT ON COLUMN bot_events.occurred_at_ms IS
    'When the source says the event happened, in Unix milliseconds.';
COMMENT ON COLUMN bot_events.received_at_ms IS
    'Admission time in Unix milliseconds; the log and rate windows are keyed on it.';
COMMENT ON COLUMN bot_events.document_ref IS
    'CAS ref of the full BotEventDocument envelope.';
COMMENT ON COLUMN bot_events.prompt_ref IS
    'CAS ref of the model-facing rendering delivered to sessions.';
COMMENT ON COLUMN bot_events.session_json IS
    'RoutedSession the event was admitted to (session id, label, ttl); NULL means the main session.';
COMMENT ON COLUMN bot_events.sender_bot_id IS
    'Sending bot for bot-originated events (self or addressed); counted against the sender rate cap.';
COMMENT ON COLUMN bot_events.hops IS
    'Federation hop count of the event; bounded by MAX_BOT_HOPS at admission.';
COMMENT ON COLUMN bot_events.reply_to_json IS
    'Private EventReplyRoute of an addressed event that asked for a receipt (asking bot and its logical session).';
COMMENT ON COLUMN bot_events.in_reply_to_json IS
    'Public BotEventReplyRef correlation of a receipt: the asked event #N at the answering bot.';
COMMENT ON COLUMN bot_events.media_json IS
    'Prepared BotEventMedia attachments appended to the run input; bytes live in the CAS.';
COMMENT ON COLUMN bot_events.tools_ref IS
    'CAS ref of receiver-bound tool declarations the routed session is created with (a chat conversation''s message_* tools).';
COMMENT ON COLUMN bot_events.notify_json IS
    'Private EventNotify delivery-receipt route of the admitting source (workflow id, kind, token).';
COMMENT ON COLUMN bot_events.outcome IS
    'Write-once delivery outcome: the model''s decision (handled|deferred|ignored|blocked) or the system''s; NULL while pending.';
COMMENT ON COLUMN bot_events.outcome_detail IS
    'Free-text detail recorded with the outcome.';
COMMENT ON COLUMN bot_events.delivery_id IS
    'Delivery that resolved the event.';
COMMENT ON COLUMN bot_events.run_id IS
    'Run that resolved the event, when one was started.';
COMMENT ON COLUMN bot_events.resolved_at_ms IS
    'When the outcome was written, in Unix milliseconds.';
