//! Bot, trigger, and event records with their store contracts. Records are
//! the wire views plus the columns the runtime keeps to itself (secrets,
//! receipt routes, the event counter); the stores are universe-scoped and
//! implemented by `store-pg` and [`crate::memory`].

use api::{
    BotDocument, BotEventMedia, BotEventOutcome, BotEventReplyRef, BotEventView, BotId,
    BotRoutedSessionView, BotTriggerDisabledReason, BotTriggerDocument, BotTriggerId,
    BotTriggerKind, BotTriggerView, BotView, PollCursorState, ProfileId,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::BotError;

// ── Bots ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotRecord {
    pub bot_id: BotId,
    pub revision: u64,
    pub document: BotDocument,
    /// Monotonic per-bot event counter; allocated at admission, shown as
    /// `#N`.
    pub event_seq: u64,
    pub closed_at_ms: Option<i64>,
    /// Sessions the controller closed on the way out, recorded by the
    /// controller itself so `bots/delete` can erase them.
    pub closed_sessions: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl BotRecord {
    pub fn is_closed(&self) -> bool {
        self.closed_at_ms.is_some()
    }

    pub fn view(&self) -> BotView {
        BotView {
            bot_id: self.bot_id.clone(),
            revision: self.revision,
            document: self.document.clone(),
            event_seq: self.event_seq,
            closed_at_ms: self.closed_at_ms,
            closed_sessions: self.closed_sessions.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

/// Roster row: the bot plus what the console needs at a glance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotRosterRow {
    pub bot: BotRecord,
    pub trigger_count: u32,
    pub pending_count: u64,
    pub last_event: Option<BotEventRecord>,
}

#[async_trait]
pub trait BotStore: Send + Sync {
    async fn create_bot(
        &self,
        bot_id: BotId,
        document: BotDocument,
        now_ms: i64,
    ) -> Result<BotRecord, BotError>;

    /// Replace the whole document and bump the revision. `expected_revision`
    /// is checked when given; a closed bot rejects everything but a change
    /// of `display_name` / `description`.
    async fn put_bot(
        &self,
        bot_id: BotId,
        document: BotDocument,
        expected_revision: Option<u64>,
        now_ms: i64,
    ) -> Result<BotRecord, BotError>;

    async fn read_bot(&self, bot_id: &BotId) -> Result<BotRecord, BotError>;

    async fn list_bots(&self) -> Result<Vec<BotRecord>, BotError>;

    /// Bots plus their trigger count, pending event count, and latest
    /// event, ordered by bot id.
    async fn list_bot_roster(&self) -> Result<Vec<BotRosterRow>, BotError>;

    /// Open (not closed) bots whose profile is `profile_id`.
    async fn list_bots_for_profile(
        &self,
        profile_id: &ProfileId,
    ) -> Result<Vec<BotRecord>, BotError>;

    /// Mark the bot closed: `closed_at_ms` set once, `enabled` cleared.
    /// Idempotent — a closed bot returns its record unchanged.
    async fn close_bot(&self, bot_id: &BotId, now_ms: i64) -> Result<BotRecord, BotError>;

    /// Union `sessions` into the bot's recorded closed sessions.
    async fn record_bot_closed_sessions(
        &self,
        bot_id: &BotId,
        sessions: Vec<String>,
    ) -> Result<Vec<String>, BotError>;

    async fn delete_bot(&self, bot_id: &BotId) -> Result<BotRecord, BotError>;

    /// Race-free `#N` allocation: increments and returns the bot's counter.
    async fn allocate_bot_event_seq(&self, bot_id: &BotId) -> Result<u64, BotError>;
}

// ── Triggers ────────────────────────────────────────────────────────────────

/// Server-minted trigger secrets, kept beside the document and shown only
/// to managing principals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotTriggerSecrets {
    /// Webhook triggers: the URL path token (24 random bytes, hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_token: Option<String>,
    /// Chat triggers with `pairing: code`: the pairing code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotTriggerRecord {
    pub bot_id: BotId,
    pub trigger_id: BotTriggerId,
    pub revision: u64,
    pub document: BotTriggerDocument,
    pub secrets: BotTriggerSecrets,
    pub disabled_reason: Option<BotTriggerDisabledReason>,
    pub disabled_at_ms: Option<i64>,
    pub last_filter_error: Option<String>,
    pub last_filter_error_at_ms: Option<i64>,
    pub cursor: Option<PollCursorState>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl BotTriggerRecord {
    pub fn kind(&self) -> BotTriggerKind {
        self.document.spec.kind()
    }

    pub fn enabled(&self) -> bool {
        self.document.enabled
    }

    /// Whether this trigger is driven by a Temporal Schedule.
    pub fn has_schedule(&self) -> bool {
        matches!(self.kind(), BotTriggerKind::Schedule | BotTriggerKind::Poll)
    }

    /// The wire view. `redact` hides the ingest path and pairing code
    /// (non-managing principals); `ingest_path` is the webhook route when
    /// the caller can build one.
    pub fn view(&self, redact: bool, ingest_path: Option<String>) -> BotTriggerView {
        BotTriggerView {
            bot_id: self.bot_id.clone(),
            trigger_id: self.trigger_id.clone(),
            revision: self.revision,
            document: self.document.clone(),
            disabled_reason: self.disabled_reason,
            disabled_at_ms: self.disabled_at_ms,
            last_filter_error: self.last_filter_error.clone(),
            last_filter_error_at_ms: self.last_filter_error_at_ms,
            cursor: self.cursor.clone(),
            ingest_path: if redact { None } else { ingest_path },
            pairing_code: if redact {
                None
            } else {
                self.secrets.pairing_code.clone()
            },
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

/// What a put writes: the document and the secrets that go with it. The
/// caller (the service) carries existing secrets forward across edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotTriggerWrite {
    pub trigger_id: BotTriggerId,
    pub document: BotTriggerDocument,
    pub secrets: BotTriggerSecrets,
    /// A spec edit of a poll trigger resets the cursor; `Some(None)` clears
    /// it, `None` leaves it as stored.
    pub cursor: Option<Option<PollCursorState>>,
}

#[async_trait]
pub trait BotTriggerStore: Send + Sync {
    /// Create when absent, otherwise replace the document and secrets and
    /// bump the revision. A replaced trigger keeps its incidents unless the
    /// document enables it again, which clears `disabled_reason`.
    async fn put_bot_trigger(
        &self,
        bot_id: &BotId,
        write: BotTriggerWrite,
        expected_revision: Option<u64>,
        now_ms: i64,
    ) -> Result<BotTriggerRecord, BotError>;

    async fn read_bot_trigger(
        &self,
        bot_id: &BotId,
        trigger_id: &BotTriggerId,
    ) -> Result<BotTriggerRecord, BotError>;

    /// The bot's triggers ordered by trigger id.
    async fn list_bot_triggers(&self, bot_id: &BotId) -> Result<Vec<BotTriggerRecord>, BotError>;

    /// Every trigger of the universe with the given kind, ordered by bot id
    /// then trigger id (schedule reconciliation, chat trigger candidates).
    async fn list_bot_triggers_by_kind(
        &self,
        kind: BotTriggerKind,
    ) -> Result<Vec<BotTriggerRecord>, BotError>;

    async fn delete_bot_trigger(
        &self,
        bot_id: &BotId,
        trigger_id: &BotTriggerId,
    ) -> Result<BotTriggerRecord, BotError>;

    /// Runtime disable (breaker, poll failures, one-shot, bot close): sets
    /// `enabled = false` with the reason. Re-enabling is a document put.
    async fn disable_bot_trigger(
        &self,
        bot_id: &BotId,
        trigger_id: &BotTriggerId,
        reason: BotTriggerDisabledReason,
        now_ms: i64,
    ) -> Result<BotTriggerRecord, BotError>;

    /// Disable every enabled trigger of the bot; returns the ones changed.
    async fn disable_bot_triggers(
        &self,
        bot_id: &BotId,
        reason: BotTriggerDisabledReason,
        now_ms: i64,
    ) -> Result<Vec<BotTriggerRecord>, BotError>;

    /// Record (or clear, with `None`) the last runtime filter failure.
    async fn set_bot_trigger_filter_error(
        &self,
        bot_id: &BotId,
        trigger_id: &BotTriggerId,
        error: Option<String>,
        now_ms: i64,
    ) -> Result<(), BotError>;

    async fn set_bot_trigger_cursor(
        &self,
        bot_id: &BotId,
        trigger_id: &BotTriggerId,
        cursor: Option<PollCursorState>,
    ) -> Result<(), BotError>;
}

// ── Events ──────────────────────────────────────────────────────────────────

/// Retention of a routed session, decided at admission from the trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RoutedSessionTtl {
    /// Inherit the bot's `routedSessionTtlMs`.
    #[default]
    Inherit,
    /// Never close (chat).
    Never,
    After {
        ms: u64,
    },
}

/// Routing target computed at admission; absent means the main session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutedSession {
    pub session_id: String,
    pub label: String,
    #[serde(default)]
    pub ttl: RoutedSessionTtl,
}

impl RoutedSession {
    pub fn view(&self) -> BotRoutedSessionView {
        BotRoutedSessionView {
            session_id: self.session_id.clone(),
            label: self.label.clone(),
        }
    }
}

/// Private return route of an addressed event that asked for a receipt:
/// the asking bot and its logical session (base id, never a generation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReplyRoute {
    pub bot_id: BotId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<RoutedSession>,
}

/// Private delivery-receipt route of the admitting source: a workflow
/// endpoint signalled `started` / `finished` with the caller's token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventNotify {
    pub workflow_id: String,
    pub workflow_kind: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotEventRecord {
    pub bot_id: BotId,
    pub event_id: String,
    pub seq: u64,
    pub trigger_id: Option<BotTriggerId>,
    pub kind: String,
    pub source: String,
    pub summary: String,
    pub occurred_at_ms: i64,
    pub received_at_ms: i64,
    /// CAS ref of the envelope document.
    pub document_ref: String,
    /// CAS ref of the model-facing rendering delivered to sessions.
    pub prompt_ref: Option<String>,
    pub session: Option<RoutedSession>,
    pub sender_bot_id: Option<BotId>,
    pub hops: u32,
    pub reply_to: Option<EventReplyRoute>,
    pub in_reply_to: Option<BotEventReplyRef>,
    pub media: Vec<BotEventMedia>,
    /// CAS ref of receiver-bound tool declarations the routed session is
    /// created with (a chat conversation's `message_*` tools).
    pub tools_ref: Option<String>,
    pub notify: Option<EventNotify>,
    pub outcome: Option<BotEventOutcome>,
    pub outcome_detail: Option<String>,
    pub delivery_id: Option<String>,
    pub run_id: Option<String>,
    pub resolved_at_ms: Option<i64>,
}

impl BotEventRecord {
    pub fn is_pending(&self) -> bool {
        self.outcome.is_none()
    }

    pub fn view(&self) -> BotEventView {
        BotEventView {
            seq: self.seq,
            event_id: self.event_id.clone(),
            trigger_id: self.trigger_id.clone(),
            kind: self.kind.clone(),
            source: self.source.clone(),
            summary: self.summary.clone(),
            occurred_at_ms: self.occurred_at_ms,
            received_at_ms: self.received_at_ms,
            document_ref: self.document_ref.clone(),
            prompt_ref: self.prompt_ref.clone(),
            session: self.session.as_ref().map(RoutedSession::view),
            sender_bot_id: self.sender_bot_id.clone(),
            hops: self.hops,
            in_reply_to: self.in_reply_to.clone(),
            media: self.media.clone(),
            outcome: self.outcome,
            outcome_detail: self.outcome_detail.clone(),
            delivery_id: self.delivery_id.clone(),
            run_id: self.run_id.clone(),
            resolved_at_ms: self.resolved_at_ms,
        }
    }
}

/// Result of an insert keyed by `(bot_id, event_id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertBotEventOutcome {
    Inserted(BotEventRecord),
    /// The id was already stored; the stored row (with its own `seq` and
    /// refs) is returned so `#N` stays stable.
    Duplicate(BotEventRecord),
}

impl InsertBotEventOutcome {
    pub fn record(&self) -> &BotEventRecord {
        match self {
            Self::Inserted(record) | Self::Duplicate(record) => record,
        }
    }

    pub fn is_duplicate(&self) -> bool {
        matches!(self, Self::Duplicate(_))
    }
}

/// Keyset cursor of the event log: newest first, `(received_at_ms, seq)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BotEventCursor {
    pub received_at_ms: i64,
    pub seq: u64,
}

/// Which events count toward a rate window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BotEventRateScope<'a> {
    /// Events admitted through one trigger.
    Trigger {
        bot_id: &'a BotId,
        trigger_id: &'a BotTriggerId,
    },
    /// Events a bot sent (self or addressed), anywhere in the universe.
    Sender { sender_bot_id: &'a BotId },
}

/// Write-once outcome of a finished delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotEventOutcomeWrite {
    pub outcome: BotEventOutcome,
    pub detail: Option<String>,
    pub delivery_id: Option<String>,
    pub run_id: Option<String>,
    pub resolved_at_ms: i64,
}

#[async_trait]
pub trait BotEventStore: Send + Sync {
    /// Insert the row; a conflict on `(bot_id, event_id)` returns the stored
    /// row instead.
    async fn insert_bot_event(
        &self,
        record: BotEventRecord,
    ) -> Result<InsertBotEventOutcome, BotError>;

    /// Wake-failure compensation: remove a row this caller inserted.
    async fn delete_bot_event(&self, bot_id: &BotId, event_id: &str) -> Result<bool, BotError>;

    async fn read_bot_event_by_seq(
        &self,
        bot_id: &BotId,
        seq: u64,
    ) -> Result<BotEventRecord, BotError>;

    async fn read_bot_event(
        &self,
        bot_id: &BotId,
        event_id: &str,
    ) -> Result<BotEventRecord, BotError>;

    async fn read_bot_events(
        &self,
        bot_id: &BotId,
        event_ids: &[String],
    ) -> Result<Vec<BotEventRecord>, BotError>;

    /// Newest first; `before` continues a page.
    async fn list_bot_events(
        &self,
        bot_id: &BotId,
        limit: usize,
        before: Option<BotEventCursor>,
    ) -> Result<Vec<BotEventRecord>, BotError>;

    async fn count_bot_events_since(
        &self,
        scope: BotEventRateScope<'_>,
        since_ms: i64,
    ) -> Result<u64, BotError>;

    /// Set the outcome columns of every listed event whose outcome is still
    /// null; returns how many rows changed.
    async fn record_bot_event_outcomes(
        &self,
        bot_id: &BotId,
        event_ids: &[String],
        write: BotEventOutcomeWrite,
    ) -> Result<u64, BotError>;
}

/// Everything the bots subsystem needs from storage, in one bound.
pub trait BotRegistryStore: BotStore + BotTriggerStore + BotEventStore {}

impl<T: BotStore + BotTriggerStore + BotEventStore> BotRegistryStore for T {}
