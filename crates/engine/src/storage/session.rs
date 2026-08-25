//! Session event-log storage contract.

use crate::{
    CORE_AGENT_LIFECYCLE_CLOSED_EVENT_KIND, CORE_AGENT_LIFECYCLE_OPENED_EVENT_KIND, CoreAgentCodec,
    CoreAgentEvent, SubagentLimits, WorkflowToolConfigEvent,
    session::{EventSeq, SessionId, SessionPosition, StoredSessionEntry, UncommittedStoredEvent},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: SessionId,
    /// Human-readable name. Store metadata only — never part of the event
    /// log or deterministic replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Cheap, durable projection of the CoreAgent lifecycle. The event log
    /// remains authoritative; stores update this alongside event appends.
    #[serde(default)]
    pub lifecycle_status: SessionLifecycleStatus,
    /// Sequence of the terminal lifecycle event. Besides auditing closure,
    /// this lets forks derive lifecycle state at their branch point without
    /// replaying inherited history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at_seq: Option<EventSeq>,
    /// Cheap catalog/list projection of immutable lifecycle ownership. This
    /// is true only when a managed-session declaration names a lifecycle
    /// controller; workflow-tool-only declarations remain ordinary sessions.
    pub managed: bool,
    pub head: Option<SessionPosition>,
    pub source_session_id: Option<SessionId>,
    pub source_seq: Option<EventSeq>,
    /// Typed provenance of a delegated (sub-agent) session. Set at creation
    /// and never changed; provenance, not ownership. `source_session_id`
    /// is clone/fork content ancestry and stays empty for profile spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOrigin {
    pub kind: SessionOriginKind,
    pub parent_session_id: SessionId,
    pub parent_run_id: u64,
    /// The tree root: the nearest ancestor without an origin. Root-scoped
    /// limits count every session whose origin names this root.
    pub root_session_id: SessionId,
    /// Absolute depth from the root (a root's direct child is depth 1).
    pub depth: u32,
    pub invocation_id: String,
    pub profile_id: String,
    pub profile_revision: u64,
    /// Effective limits pinned at spawn: the parent's grant attenuated by
    /// the parent's own origin limits.
    pub limits: SubagentLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOriginKind {
    Subagent,
}

/// Which root-scoped limit a session creation exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOriginLimit {
    MaxDepth,
    MaxDescendants,
    MaxConcurrent,
}

impl std::fmt::Display for SessionOriginLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MaxDepth => "maxDepth",
            Self::MaxDescendants => "maxDescendants",
            Self::MaxConcurrent => "maxConcurrent",
        })
    }
}

/// Root-scoped counts a store checks before reserving a delegated session.
/// `descendants` is lifetime (closed sessions included); `open_descendants`
/// excludes closed ones. Both exclude the root itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionOriginCounts {
    pub descendants: u64,
    pub open_descendants: u64,
}

/// Shared reservation rule for every session store: the child row is the
/// reservation, so this runs inside the store's creation transaction with
/// the root row locked.
pub fn check_origin_limits(
    origin: &SessionOrigin,
    counts: SessionOriginCounts,
) -> Result<(), SessionStoreError> {
    let exceeded = |limit, max: u64, actual: u64| SessionStoreError::OriginLimitExceeded {
        root_session_id: origin.root_session_id.clone(),
        limit,
        max,
        actual,
    };
    if u64::from(origin.depth) > u64::from(origin.limits.max_depth) {
        return Err(exceeded(
            SessionOriginLimit::MaxDepth,
            u64::from(origin.limits.max_depth),
            u64::from(origin.depth),
        ));
    }
    if counts.descendants >= u64::from(origin.limits.max_descendants) {
        return Err(exceeded(
            SessionOriginLimit::MaxDescendants,
            u64::from(origin.limits.max_descendants),
            counts.descendants,
        ));
    }
    if counts.open_descendants >= u64::from(origin.limits.max_concurrent) {
        return Err(exceeded(
            SessionOriginLimit::MaxConcurrent,
            u64::from(origin.limits.max_concurrent),
            counts.open_descendants,
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleStatus {
    #[default]
    New,
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSession {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Delegation provenance. Stores treat the creation as the root-scoped
    /// reservation: the parent and root must exist and the origin's limits
    /// must hold, atomically with the insert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateClonedSession {
    pub source_session_id: SessionId,
    pub session_id: SessionId,
    pub created_at_ms: u64,
    #[serde(default)]
    pub opening_events: Vec<UncommittedStoredEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateForkedSession {
    pub source_session_id: SessionId,
    pub session_id: SessionId,
    /// Branch point in the source session's effective log. `0` means an empty
    /// inherited prefix; the child then appends from seq 1.
    pub source_seq: EventSeq,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendSessionEvents {
    pub session_id: SessionId,
    pub expected_head: Option<SessionPosition>,
    pub events: Vec<UncommittedStoredEvent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendSessionEventsResult {
    pub entries: Vec<StoredSessionEntry>,
    pub head: Option<SessionPosition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSessionEvents {
    pub session_id: SessionId,
    pub after: Option<EventSeq>,
    pub limit: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPage {
    pub entries: Vec<StoredSessionEntry>,
    pub next_after: Option<EventSeq>,
    pub complete: bool,
}

/// Keyset cursor for [`SessionStore::list_sessions`]: the sort key of the
/// last row of the previous page (most-recently-updated-first ordering).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListCursor {
    pub updated_at_ms: u64,
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<SessionListCursor>,
    pub limit: usize,
    /// Only sessions whose origin names this root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_session_id: Option<SessionId>,
    /// Only sessions whose origin names this parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListPage {
    pub sessions: Vec<SessionRecord>,
    /// Present when more rows exist past this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<SessionListCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionStoreError {
    #[error("session already exists: {session_id}")]
    SessionAlreadyExists { session_id: SessionId },

    #[error("session not found: {session_id}")]
    SessionNotFound { session_id: SessionId },

    #[error("expected head mismatch for {session_id}: expected {expected:?}, actual {actual:?}")]
    ExpectedHeadMismatch {
        session_id: SessionId,
        expected: Option<SessionPosition>,
        actual: Option<SessionPosition>,
    },

    #[error("invalid page limit: {limit}")]
    InvalidLimit { limit: usize },

    #[error("invalid fork point for {session_id} at seq {source_seq}: {message}")]
    InvalidForkPoint {
        session_id: SessionId,
        source_seq: EventSeq,
        message: String,
    },

    #[error("session origin limit {limit} exceeded for root {root_session_id}: {actual} of {max}")]
    OriginLimitExceeded {
        root_session_id: SessionId,
        limit: SessionOriginLimit,
        max: u64,
        actual: u64,
    },

    #[error("session is not closed: {session_id} ({lifecycle_status:?})")]
    SessionNotClosed {
        session_id: SessionId,
        lifecycle_status: SessionLifecycleStatus,
    },

    #[error("session has fork children and still backs their inherited history: {session_id}")]
    SessionHasForkChildren { session_id: SessionId },

    #[error("lifecycle-managed session cannot be cloned or forked: {session_id}")]
    ManagedSessionCannotBranch { session_id: SessionId },

    #[error("session store failure: {message}")]
    Store { message: String },
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<SessionRecord, SessionStoreError>;

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, SessionStoreError>;

    /// Page session records for this store's scope, most recently updated
    /// first with session id as the tie-break, both descending.
    async fn list_sessions(
        &self,
        request: ListSessions,
    ) -> Result<SessionListPage, SessionStoreError> {
        let _ = request;
        Err(SessionStoreError::Store {
            message: "list_sessions is not supported by this session store".to_owned(),
        })
    }

    /// Replace the session's display name (`None` clears it). Metadata only:
    /// does not touch the event log or `updated_at_ms`.
    async fn set_session_display_name(
        &self,
        session_id: &SessionId,
        display_name: Option<String>,
    ) -> Result<SessionRecord, SessionStoreError> {
        let _ = display_name;
        Err(SessionStoreError::Store {
            message: format!(
                "set_session_display_name is not supported by this session store for {session_id}"
            ),
        })
    }

    /// Delete a logically closed session. Implementations must perform the
    /// lifecycle check and delete atomically and reject sessions whose event
    /// history is still inherited by a fork child.
    async fn delete_closed_session(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionRecord, SessionStoreError> {
        Err(SessionStoreError::Store {
            message: format!(
                "delete_closed_session is not supported by this session store for {session_id}"
            ),
        })
    }

    async fn create_cloned_session(
        &self,
        request: CreateClonedSession,
    ) -> Result<SessionRecord, SessionStoreError> {
        Err(SessionStoreError::Store {
            message: format!(
                "create_cloned_session is not supported by this session store for {}",
                request.session_id
            ),
        })
    }

    async fn create_forked_session(
        &self,
        request: CreateForkedSession,
    ) -> Result<SessionRecord, SessionStoreError> {
        Err(SessionStoreError::Store {
            message: format!(
                "create_forked_session is not supported by this session store for {}",
                request.session_id
            ),
        })
    }

    async fn safe_fork_seq(&self, session_id: &SessionId) -> Result<EventSeq, SessionStoreError> {
        Err(SessionStoreError::Store {
            message: format!(
                "safe_fork_seq is not supported by this session store for {session_id}"
            ),
        })
    }

    async fn append(
        &self,
        request: AppendSessionEvents,
    ) -> Result<AppendSessionEventsResult, SessionStoreError>;

    async fn read_after(
        &self,
        request: ReadSessionEvents,
    ) -> Result<SessionPage, SessionStoreError>;

    async fn head(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionPosition>, SessionStoreError>;
}

#[derive(Clone, Default)]
pub struct InMemorySessionStore {
    inner: Arc<RwLock<InMemorySessionStoreInner>>,
}

#[derive(Default)]
struct InMemorySessionStoreInner {
    records: BTreeMap<SessionId, SessionRecord>,
    entries: BTreeMap<SessionId, Vec<StoredSessionEntry>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<SessionRecord, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        if inner.records.contains_key(&request.session_id) {
            return Err(SessionStoreError::SessionAlreadyExists {
                session_id: request.session_id,
            });
        }
        if let Some(origin) = &request.origin {
            for session_id in [&origin.parent_session_id, &origin.root_session_id] {
                if !inner.records.contains_key(session_id) {
                    return Err(SessionStoreError::SessionNotFound {
                        session_id: session_id.clone(),
                    });
                }
            }
            check_origin_limits(origin, in_memory_origin_counts(&inner, &origin.root_session_id))?;
        }
        let record = SessionRecord {
            session_id: request.session_id,
            display_name: request.display_name,
            lifecycle_status: SessionLifecycleStatus::New,
            closed_at_seq: None,
            managed: false,
            head: None,
            source_session_id: None,
            source_seq: None,
            origin: request.origin,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        inner.entries.insert(record.session_id.clone(), Vec::new());
        inner
            .records
            .insert(record.session_id.clone(), record.clone());
        Ok(record)
    }

    async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, SessionStoreError> {
        let inner = self.inner.read().map_err(|_| SessionStoreError::Store {
            message: "session store read lock poisoned".into(),
        })?;
        Ok(inner.records.get(session_id).cloned())
    }

    async fn list_sessions(
        &self,
        request: ListSessions,
    ) -> Result<SessionListPage, SessionStoreError> {
        if request.limit == 0 {
            return Err(SessionStoreError::InvalidLimit { limit: 0 });
        }
        let inner = self.inner.read().map_err(|_| SessionStoreError::Store {
            message: "session store read lock poisoned".into(),
        })?;
        let mut records: Vec<&SessionRecord> = inner.records.values().collect();
        records.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| right.session_id.cmp(&left.session_id))
        });
        let mut sessions: Vec<SessionRecord> = records
            .into_iter()
            .filter(|record| {
                request.root_session_id.as_ref().is_none_or(|root| {
                    record
                        .origin
                        .as_ref()
                        .is_some_and(|origin| &origin.root_session_id == root)
                })
            })
            .filter(|record| {
                request.parent_session_id.as_ref().is_none_or(|parent| {
                    record
                        .origin
                        .as_ref()
                        .is_some_and(|origin| &origin.parent_session_id == parent)
                })
            })
            .filter(|record| {
                request.cursor.as_ref().is_none_or(|cursor| {
                    (record.updated_at_ms, record.session_id.as_str())
                        < (cursor.updated_at_ms, cursor.session_id.as_str())
                })
            })
            .take(request.limit.saturating_add(1))
            .cloned()
            .collect();
        let next_cursor = (sessions.len() > request.limit).then(|| {
            sessions.truncate(request.limit);
            let last = sessions.last().expect("non-empty page");
            SessionListCursor {
                updated_at_ms: last.updated_at_ms,
                session_id: last.session_id.clone(),
            }
        });
        Ok(SessionListPage {
            sessions,
            next_cursor,
        })
    }

    async fn set_session_display_name(
        &self,
        session_id: &SessionId,
        display_name: Option<String>,
    ) -> Result<SessionRecord, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        let record = inner.records.get_mut(session_id).ok_or_else(|| {
            SessionStoreError::SessionNotFound {
                session_id: session_id.clone(),
            }
        })?;
        record.display_name = display_name;
        Ok(record.clone())
    }

    async fn delete_closed_session(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionRecord, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        let record = inner.records.get(session_id).cloned().ok_or_else(|| {
            SessionStoreError::SessionNotFound {
                session_id: session_id.clone(),
            }
        })?;
        if record.lifecycle_status != SessionLifecycleStatus::Closed {
            return Err(SessionStoreError::SessionNotClosed {
                session_id: session_id.clone(),
                lifecycle_status: record.lifecycle_status,
            });
        }
        if inner.records.values().any(|candidate| {
            candidate.source_session_id.as_ref() == Some(session_id)
                && candidate.source_seq.is_some()
        }) {
            return Err(SessionStoreError::SessionHasForkChildren {
                session_id: session_id.clone(),
            });
        }

        inner.records.remove(session_id);
        inner.entries.remove(session_id);
        for candidate in inner.records.values_mut() {
            if candidate.source_session_id.as_ref() == Some(session_id) {
                candidate.source_session_id = None;
            }
        }
        Ok(record)
    }

    async fn create_cloned_session(
        &self,
        request: CreateClonedSession,
    ) -> Result<SessionRecord, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        let source = inner
            .records
            .get(&request.source_session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: request.source_session_id.clone(),
            })?;
        if source.managed {
            return Err(SessionStoreError::ManagedSessionCannotBranch {
                session_id: request.source_session_id,
            });
        }
        if inner.records.contains_key(&request.session_id) {
            return Err(SessionStoreError::SessionAlreadyExists {
                session_id: request.session_id,
            });
        }

        let mut record = SessionRecord {
            session_id: request.session_id,
            display_name: None,
            lifecycle_status: SessionLifecycleStatus::New,
            closed_at_seq: None,
            managed: false,
            head: None,
            source_session_id: Some(request.source_session_id),
            source_seq: None,
            origin: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        let committed = commit_uncommitted_events(&mut record, request.opening_events);
        inner.entries.insert(record.session_id.clone(), committed);
        inner
            .records
            .insert(record.session_id.clone(), record.clone());
        Ok(record)
    }

    async fn create_forked_session(
        &self,
        request: CreateForkedSession,
    ) -> Result<SessionRecord, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        let source = inner
            .records
            .get(&request.source_session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: request.source_session_id.clone(),
            })?;
        if source.managed {
            return Err(SessionStoreError::ManagedSessionCannotBranch {
                session_id: request.source_session_id,
            });
        }
        if inner.records.contains_key(&request.session_id) {
            return Err(SessionStoreError::SessionAlreadyExists {
                session_id: request.session_id,
            });
        }
        validate_in_memory_fork_point(&inner, &request.source_session_id, request.source_seq)?;
        let source = inner
            .records
            .get(&request.source_session_id)
            .expect("validated source session");
        let (lifecycle_status, closed_at_seq) = lifecycle_at_fork(source, request.source_seq);
        let head = position_from_nonzero_seq(request.source_seq);
        let record = SessionRecord {
            session_id: request.session_id,
            display_name: None,
            lifecycle_status,
            closed_at_seq,
            managed: false,
            head,
            source_session_id: Some(request.source_session_id),
            source_seq: Some(request.source_seq),
            origin: None,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        inner.entries.insert(record.session_id.clone(), Vec::new());
        inner
            .records
            .insert(record.session_id.clone(), record.clone());
        Ok(record)
    }

    async fn safe_fork_seq(&self, session_id: &SessionId) -> Result<EventSeq, SessionStoreError> {
        let inner = self.inner.read().map_err(|_| SessionStoreError::Store {
            message: "session store read lock poisoned".into(),
        })?;
        let entries = effective_entries(&inner, session_id)?;
        Ok(largest_safe_fork_seq(
            &entries,
            effective_head_u64(&inner, session_id)?,
        ))
    }

    async fn append(
        &self,
        request: AppendSessionEvents,
    ) -> Result<AppendSessionEventsResult, SessionStoreError> {
        let mut inner = self.inner.write().map_err(|_| SessionStoreError::Store {
            message: "session store write lock poisoned".into(),
        })?;
        let actual_head = inner
            .records
            .get(&request.session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: request.session_id.clone(),
            })?
            .head
            .clone();
        if request.expected_head != actual_head {
            return Err(SessionStoreError::ExpectedHeadMismatch {
                session_id: request.session_id,
                expected: request.expected_head,
                actual: actual_head,
            });
        }

        let record = inner
            .records
            .get_mut(&request.session_id)
            .expect("validated session record");
        let committed = commit_uncommitted_events(record, request.events);
        let head = record.head.clone();

        let entries = inner
            .entries
            .get_mut(&request.session_id)
            .expect("session entries exist for record");
        entries.extend(committed.clone());

        Ok(AppendSessionEventsResult {
            entries: committed,
            head,
        })
    }

    async fn read_after(
        &self,
        request: ReadSessionEvents,
    ) -> Result<SessionPage, SessionStoreError> {
        if request.limit == 0 {
            return Err(SessionStoreError::InvalidLimit { limit: 0 });
        }
        let inner = self.inner.read().map_err(|_| SessionStoreError::Store {
            message: "session store read lock poisoned".into(),
        })?;
        let entries = effective_entries(&inner, &request.session_id)?;
        let mut selected = entries
            .iter()
            .filter(|entry| request.after.is_none_or(|after| entry.position.seq > after))
            .take(request.limit.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        let complete = selected.len() <= request.limit;
        if !complete {
            selected.truncate(request.limit);
        }
        let next_after = selected.last().map(|entry| entry.position.seq);
        Ok(SessionPage {
            entries: selected,
            next_after,
            complete,
        })
    }

    async fn head(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionPosition>, SessionStoreError> {
        let inner = self.inner.read().map_err(|_| SessionStoreError::Store {
            message: "session store read lock poisoned".into(),
        })?;
        Ok(inner
            .records
            .get(session_id)
            .and_then(|record| record.head.clone()))
    }
}

fn in_memory_origin_counts(
    inner: &InMemorySessionStoreInner,
    root_session_id: &SessionId,
) -> SessionOriginCounts {
    let mut counts = SessionOriginCounts::default();
    for record in inner.records.values() {
        if record
            .origin
            .as_ref()
            .is_some_and(|origin| &origin.root_session_id == root_session_id)
        {
            counts.descendants += 1;
            if record.lifecycle_status != SessionLifecycleStatus::Closed {
                counts.open_descendants += 1;
            }
        }
    }
    counts
}

pub fn largest_safe_fork_seq(entries: &[StoredSessionEntry], head: u64) -> EventSeq {
    let ranges = core_run_ranges(entries);
    let earliest_open = ranges
        .values()
        .filter(|range| range.terminal_seq.is_none())
        .map(|range| range.first_seq)
        .min();
    EventSeq::new(earliest_open.map_or(head, |seq| seq.saturating_sub(1)))
}

pub fn validate_fork_point(
    session_id: &SessionId,
    source_seq: EventSeq,
    entries: &[StoredSessionEntry],
    head: u64,
) -> Result<(), SessionStoreError> {
    let source_seq_u64 = source_seq.as_u64();
    if source_seq_u64 > head {
        return Err(SessionStoreError::InvalidForkPoint {
            session_id: session_id.clone(),
            source_seq,
            message: format!("source seq is beyond session head {head}"),
        });
    }
    for range in core_run_ranges(entries).values() {
        let end_exclusive = range.terminal_seq.map_or(head.saturating_add(1), |seq| seq);
        if source_seq_u64 >= range.first_seq && source_seq_u64 < end_exclusive {
            return Err(SessionStoreError::InvalidForkPoint {
                session_id: session_id.clone(),
                source_seq,
                message: format!(
                    "seq is inside non-terminal run {} ({}..{})",
                    range.run_id,
                    range.first_seq,
                    range
                        .terminal_seq
                        .map_or_else(|| "head".to_owned(), |seq| seq.to_string())
                ),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct RunRange {
    run_id: u64,
    first_seq: u64,
    terminal_seq: Option<u64>,
}

fn core_run_ranges(entries: &[StoredSessionEntry]) -> BTreeMap<u64, RunRange> {
    let mut ranges = BTreeMap::new();
    for entry in entries {
        let Some(boundary) = run_boundary(entry) else {
            continue;
        };
        let range = ranges.entry(boundary.run_id).or_insert_with(|| RunRange {
            run_id: boundary.run_id,
            first_seq: entry.position.seq.as_u64(),
            terminal_seq: None,
        });
        range.first_seq = range.first_seq.min(entry.position.seq.as_u64());
        if boundary.terminal {
            range.terminal_seq = Some(entry.position.seq.as_u64());
        }
    }
    ranges
}

#[derive(Clone, Copy, Debug)]
struct RunBoundary {
    run_id: u64,
    terminal: bool,
}

fn run_boundary(entry: &StoredSessionEntry) -> Option<RunBoundary> {
    let run_id = entry
        .joins
        .get("run_id")
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| {
            entry
                .event
                .payload
                .get("kind")
                .and_then(|kind| kind.get("run_id"))
                .and_then(serde_json::Value::as_u64)
        })?;
    let terminal = matches!(
        entry.event.kind.as_str(),
        "lightspeed.core.run.completed"
            | "lightspeed.core.run.failed"
            | "lightspeed.core.run.cancelled"
            | "lightspeed.core.run.force_cancelled"
            | "lightspeed.core.run.queued_cancelled"
    );
    let is_run = terminal
        || matches!(
            entry.event.kind.as_str(),
            "lightspeed.core.run.accepted"
                | "lightspeed.core.run.started"
                | "lightspeed.core.run.steering_accepted"
                | "lightspeed.core.run.cancellation_requested"
        );
    is_run.then_some(RunBoundary { run_id, terminal })
}

fn commit_uncommitted_events(
    record: &mut SessionRecord,
    events: Vec<UncommittedStoredEvent>,
) -> Vec<StoredSessionEntry> {
    let mut committed = Vec::with_capacity(events.len());
    for event in events {
        let next_seq = EventSeq::new(
            record
                .head
                .as_ref()
                .map_or(1, |position| position.seq.as_u64().saturating_add(1)),
        );
        let position = SessionPosition { seq: next_seq };
        let entry = StoredSessionEntry {
            position: position.clone(),
            observed_at_ms: event.observed_at_ms,
            joins: event.joins,
            event: event.event,
        };
        apply_lifecycle_projection(record, &entry);
        record.head = Some(position);
        record.updated_at_ms = entry.observed_at_ms;
        committed.push(entry);
    }
    committed
}

pub fn apply_lifecycle_projection(record: &mut SessionRecord, entry: &StoredSessionEntry) {
    match entry.event.kind.as_str() {
        CORE_AGENT_LIFECYCLE_OPENED_EVENT_KIND => {
            record.lifecycle_status = SessionLifecycleStatus::Open;
            record.closed_at_seq = None;
        }
        CORE_AGENT_LIFECYCLE_CLOSED_EVENT_KIND => {
            record.lifecycle_status = SessionLifecycleStatus::Closed;
            record.closed_at_seq = Some(entry.position.seq);
        }
        "lightspeed.core.workflow_tool_config.managed_bindings_admitted" => {
            if matches!(
                CoreAgentCodec.decode_event(&entry.event),
                Ok(CoreAgentEvent::WorkflowToolConfig(
                    WorkflowToolConfigEvent::ManagedBindingsAdmitted {
                        lifecycle_controller: Some(_),
                        ..
                    }
                ))
            ) {
                record.managed = true;
            }
        }
        _ => {}
    }
}

pub fn lifecycle_at_fork(
    source: &SessionRecord,
    source_seq: EventSeq,
) -> (SessionLifecycleStatus, Option<EventSeq>) {
    if source_seq.as_u64() == 0 {
        return (SessionLifecycleStatus::New, None);
    }
    if let Some(closed_at_seq) = source.closed_at_seq
        && closed_at_seq <= source_seq
    {
        return (SessionLifecycleStatus::Closed, Some(closed_at_seq));
    }
    (SessionLifecycleStatus::Open, None)
}

fn effective_head_u64(
    inner: &InMemorySessionStoreInner,
    session_id: &SessionId,
) -> Result<u64, SessionStoreError> {
    inner
        .records
        .get(session_id)
        .ok_or_else(|| SessionStoreError::SessionNotFound {
            session_id: session_id.clone(),
        })
        .map(|record| record.head.as_ref().map_or(0, |head| head.seq.as_u64()))
}

fn effective_entries(
    inner: &InMemorySessionStoreInner,
    session_id: &SessionId,
) -> Result<Vec<StoredSessionEntry>, SessionStoreError> {
    let head = effective_head_u64(inner, session_id)?;
    effective_entries_up_to(inner, session_id, head)
}

fn effective_entries_up_to(
    inner: &InMemorySessionStoreInner,
    session_id: &SessionId,
    max_seq: u64,
) -> Result<Vec<StoredSessionEntry>, SessionStoreError> {
    let record =
        inner
            .records
            .get(session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: session_id.clone(),
            })?;

    if let (Some(source_session_id), Some(source_seq)) =
        (&record.source_session_id, record.source_seq)
    {
        let branch_seq = source_seq.as_u64();
        if max_seq <= branch_seq {
            return effective_entries_up_to(inner, source_session_id, max_seq);
        }
        let mut entries = effective_entries_up_to(inner, source_session_id, branch_seq)?;
        entries.extend(local_entries_up_to(inner, session_id, branch_seq, max_seq)?);
        return Ok(entries);
    }

    local_entries_up_to(inner, session_id, 0, max_seq)
}

fn local_entries_up_to(
    inner: &InMemorySessionStoreInner,
    session_id: &SessionId,
    after: u64,
    max_seq: u64,
) -> Result<Vec<StoredSessionEntry>, SessionStoreError> {
    let entries =
        inner
            .entries
            .get(session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: session_id.clone(),
            })?;
    Ok(entries
        .iter()
        .filter(|entry| {
            let seq = entry.position.seq.as_u64();
            seq > after && seq <= max_seq
        })
        .cloned()
        .collect())
}

fn validate_in_memory_fork_point(
    inner: &InMemorySessionStoreInner,
    source_session_id: &SessionId,
    source_seq: EventSeq,
) -> Result<(), SessionStoreError> {
    let entries = effective_entries(inner, source_session_id)?;
    validate_fork_point(
        source_session_id,
        source_seq,
        &entries,
        effective_head_u64(inner, source_session_id)?,
    )
}

fn position_from_nonzero_seq(seq: EventSeq) -> Option<SessionPosition> {
    (seq.as_u64() > 0).then_some(SessionPosition { seq })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{StoredEvent, StoredJoins};

    fn test_event(at_ms: u64, kind: &'static str) -> UncommittedStoredEvent {
        UncommittedStoredEvent {
            observed_at_ms: at_ms,
            joins: StoredJoins::default(),
            event: StoredEvent::new(kind, 1, serde_json::Value::Object(Default::default())),
        }
    }

    fn open_event(at_ms: u64) -> UncommittedStoredEvent {
        test_event(at_ms, "lightspeed.test.lifecycle.closed")
    }

    fn lifecycle_opened_event(at_ms: u64) -> UncommittedStoredEvent {
        test_event(at_ms, CORE_AGENT_LIFECYCLE_OPENED_EVENT_KIND)
    }

    fn managed_bindings_event(
        at_ms: u64,
        lifecycle_controller: Option<crate::WorkflowEndpointRef>,
    ) -> UncommittedStoredEvent {
        CoreAgentCodec
            .encode_uncommitted(&crate::UncommittedCoreAgentEvent {
                observed_at_ms: at_ms,
                joins: crate::CoreAgentJoins::default(),
                event: CoreAgentEvent::WorkflowToolConfig(
                    WorkflowToolConfigEvent::ManagedBindingsAdmitted {
                        session_universe_id: uuid::Uuid::from_u128(1),
                        declaration_version: 1,
                        lifecycle_controller,
                        creation_fingerprint: "test-creation-fingerprint".to_owned(),
                        bindings: Vec::new(),
                    },
                ),
            })
            .expect("encode managed bindings event")
    }

    fn lifecycle_closed_event(at_ms: u64) -> UncommittedStoredEvent {
        test_event(at_ms, CORE_AGENT_LIFECYCLE_CLOSED_EVENT_KIND)
    }

    fn run_event(at_ms: u64, kind: &'static str, run_id: u64) -> UncommittedStoredEvent {
        UncommittedStoredEvent {
            observed_at_ms: at_ms,
            joins: StoredJoins::from([("run_id".to_owned(), run_id.to_string())]),
            event: StoredEvent::new(kind, 1, serde_json::Value::Object(Default::default())),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_assigns_session_local_sequences() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::new("session-a");
        store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create session");

        let result = store
            .append(AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: None,
                events: vec![open_event(10), open_event(11)],
            })
            .await
            .expect("append");

        assert_eq!(result.entries[0].position.seq, EventSeq::new(1));
        assert_eq!(result.entries[1].position.seq, EventSeq::new(2));
        assert_eq!(
            result.head.as_ref().map(|head| head.seq),
            Some(EventSeq::new(2))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_lists_newest_first_with_keyset_cursor() {
        let store = InMemorySessionStore::new();
        for (name, created_at_ms) in [("session-a", 10), ("session-b", 20), ("session-c", 20)] {
            store
                .create_session(CreateSession {
                    session_id: SessionId::new(name),
                    display_name: Some(format!("Session {name}")),
                    origin: None,
                    created_at_ms,
                })
                .await
                .expect("create session");
        }

        let first = store
            .list_sessions(ListSessions {
                cursor: None,
                limit: 2,
                root_session_id: None,
                parent_session_id: None,
            })
            .await
            .expect("first page");
        assert_eq!(
            first
                .sessions
                .iter()
                .map(|record| record.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["session-c", "session-b"]
        );
        let cursor = first.next_cursor.expect("more rows remain");

        let second = store
            .list_sessions(ListSessions {
                cursor: Some(cursor),
                limit: 2,
                root_session_id: None,
                parent_session_id: None,
            })
            .await
            .expect("second page");
        assert_eq!(
            second
                .sessions
                .iter()
                .map(|record| record.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["session-a"]
        );
        assert_eq!(
            second.sessions[0].display_name.as_deref(),
            Some("Session session-a")
        );
        assert!(second.next_cursor.is_none());

        assert!(matches!(
            store
                .list_sessions(ListSessions {
                    cursor: None,
                    limit: 0,
                    root_session_id: None,
                    parent_session_id: None,
                })
                .await,
            Err(SessionStoreError::InvalidLimit { limit: 0 })
        ));
    }

    fn subagent_origin(parent: &str, root: &str, depth: u32, limits: SubagentLimits) -> SessionOrigin {
        SessionOrigin {
            kind: SessionOriginKind::Subagent,
            parent_session_id: SessionId::new(parent),
            parent_run_id: 1,
            root_session_id: SessionId::new(root),
            depth,
            invocation_id: format!("wti:sha256:{}", "a".repeat(64)),
            profile_id: "reviewer".to_owned(),
            profile_revision: 1,
            limits,
        }
    }

    async fn create_delegated(
        store: &InMemorySessionStore,
        session_id: &str,
        origin: SessionOrigin,
    ) -> Result<SessionRecord, SessionStoreError> {
        store
            .create_session(CreateSession {
                session_id: SessionId::new(session_id),
                display_name: None,
                origin: Some(origin),
                created_at_ms: 1,
            })
            .await
    }

    async fn close_session(store: &InMemorySessionStore, session_id: &str) {
        store
            .append(AppendSessionEvents {
                session_id: SessionId::new(session_id),
                expected_head: None,
                events: vec![lifecycle_opened_event(10), lifecycle_closed_event(11)],
            })
            .await
            .expect("close session");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_reserves_delegated_sessions_against_root_limits() {
        let store = InMemorySessionStore::new();
        store
            .create_session(CreateSession {
                session_id: SessionId::new("root"),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create root");
        let limits = SubagentLimits {
            max_depth: 2,
            max_descendants: 3,
            max_concurrent: 2,
            deadline_ms: 1_000,
        };

        create_delegated(&store, "child-a", subagent_origin("root", "root", 1, limits))
            .await
            .expect("first child");
        create_delegated(&store, "child-b", subagent_origin("root", "root", 1, limits))
            .await
            .expect("second child");
        // Two open descendants: the concurrency limit refuses a third even
        // though the lifetime limit still has room.
        let concurrent = create_delegated(&store, "child-c", subagent_origin("root", "root", 1, limits))
            .await
            .expect_err("third open child");
        assert_eq!(
            concurrent,
            SessionStoreError::OriginLimitExceeded {
                root_session_id: SessionId::new("root"),
                limit: SessionOriginLimit::MaxConcurrent,
                max: 2,
                actual: 2,
            }
        );

        // Closing one frees its concurrency slot but not its lifetime slot.
        close_session(&store, "child-a").await;
        create_delegated(&store, "child-c", subagent_origin("child-b", "root", 2, limits))
            .await
            .expect("grandchild after a close");
        let descendants = create_delegated(&store, "child-d", subagent_origin("root", "root", 1, limits))
            .await
            .expect_err("fourth lifetime child");
        assert_eq!(
            descendants,
            SessionStoreError::OriginLimitExceeded {
                root_session_id: SessionId::new("root"),
                limit: SessionOriginLimit::MaxDescendants,
                max: 3,
                actual: 3,
            }
        );
        assert!(
            store
                .load_session(&SessionId::new("child-d"))
                .await
                .expect("load")
                .is_none(),
            "a refused reservation leaves no row"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_refuses_delegation_past_max_depth() {
        let store = InMemorySessionStore::new();
        store
            .create_session(CreateSession {
                session_id: SessionId::new("root"),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create root");
        let limits = SubagentLimits {
            max_depth: 1,
            ..SubagentLimits::default()
        };
        create_delegated(&store, "child", subagent_origin("root", "root", 1, limits))
            .await
            .expect("depth-1 child");
        let too_deep = create_delegated(&store, "grandchild", subagent_origin("child", "root", 2, limits))
            .await
            .expect_err("depth-2 child");
        assert_eq!(
            too_deep,
            SessionStoreError::OriginLimitExceeded {
                root_session_id: SessionId::new("root"),
                limit: SessionOriginLimit::MaxDepth,
                max: 1,
                actual: 2,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_requires_the_parent_and_root_of_a_delegated_session() {
        let store = InMemorySessionStore::new();
        store
            .create_session(CreateSession {
                session_id: SessionId::new("root"),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create root");
        let missing_parent = create_delegated(
            &store,
            "child",
            subagent_origin("ghost", "root", 1, SubagentLimits::default()),
        )
        .await
        .expect_err("missing parent");
        assert_eq!(
            missing_parent,
            SessionStoreError::SessionNotFound {
                session_id: SessionId::new("ghost"),
            }
        );
        let missing_root = create_delegated(
            &store,
            "child",
            subagent_origin("root", "ghost-root", 1, SubagentLimits::default()),
        )
        .await
        .expect_err("missing root");
        assert_eq!(
            missing_root,
            SessionStoreError::SessionNotFound {
                session_id: SessionId::new("ghost-root"),
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_lists_by_root_and_by_parent() {
        let store = InMemorySessionStore::new();
        for root in ["root-a", "root-b"] {
            store
                .create_session(CreateSession {
                    session_id: SessionId::new(root),
                    display_name: None,
                    origin: None,
                    created_at_ms: 1,
                })
                .await
                .expect("create root");
        }
        let limits = SubagentLimits::default();
        create_delegated(&store, "a-child", subagent_origin("root-a", "root-a", 1, limits))
            .await
            .expect("a child");
        create_delegated(&store, "a-grandchild", subagent_origin("a-child", "root-a", 2, limits))
            .await
            .expect("a grandchild");
        create_delegated(&store, "b-child", subagent_origin("root-b", "root-b", 1, limits))
            .await
            .expect("b child");

        let ids = |page: SessionListPage| {
            let mut ids = page
                .sessions
                .iter()
                .map(|record| record.session_id.as_str().to_owned())
                .collect::<Vec<_>>();
            ids.sort();
            ids
        };
        let under_a = store
            .list_sessions(ListSessions {
                cursor: None,
                limit: 10,
                root_session_id: Some(SessionId::new("root-a")),
                parent_session_id: None,
            })
            .await
            .expect("list by root");
        assert_eq!(ids(under_a), vec!["a-child", "a-grandchild"]);
        let children_of_a = store
            .list_sessions(ListSessions {
                cursor: None,
                limit: 10,
                root_session_id: None,
                parent_session_id: Some(SessionId::new("root-a")),
            })
            .await
            .expect("list by parent");
        assert_eq!(ids(children_of_a), vec!["a-child"]);
        let everything = store
            .list_sessions(ListSessions {
                cursor: None,
                limit: 10,
                root_session_id: None,
                parent_session_id: None,
            })
            .await
            .expect("list all");
        assert_eq!(everything.sessions.len(), 5, "roots have no origin and are listed only unfiltered");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_sets_and_clears_display_name() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::new("session-a");
        store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create session");

        let named = store
            .set_session_display_name(&session_id, Some("Family chat".to_owned()))
            .await
            .expect("set display name");
        assert_eq!(named.display_name.as_deref(), Some("Family chat"));
        assert_eq!(named.updated_at_ms, 1, "rename must not count as activity");

        let cleared = store
            .set_session_display_name(&session_id, None)
            .await
            .expect("clear display name");
        assert_eq!(cleared.display_name, None);

        assert!(matches!(
            store
                .set_session_display_name(&SessionId::new("missing"), None)
                .await,
            Err(SessionStoreError::SessionNotFound { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lifecycle_projection_drives_forks_and_closed_only_deletion() {
        let store = InMemorySessionStore::new();
        let parent = SessionId::new("parent");
        store
            .create_session(CreateSession {
                session_id: parent.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create parent");

        let not_closed = store
            .delete_closed_session(&parent)
            .await
            .expect_err("new session cannot be deleted");
        assert!(matches!(
            not_closed,
            SessionStoreError::SessionNotClosed {
                lifecycle_status: SessionLifecycleStatus::New,
                ..
            }
        ));

        store
            .append(AppendSessionEvents {
                session_id: parent.clone(),
                expected_head: None,
                events: vec![
                    lifecycle_opened_event(10),
                    test_event(11, "parent.work"),
                    lifecycle_closed_event(12),
                ],
            })
            .await
            .expect("close parent");
        let parent_record = store
            .load_session(&parent)
            .await
            .expect("load parent")
            .expect("parent exists");
        assert_eq!(
            parent_record.lifecycle_status,
            SessionLifecycleStatus::Closed
        );
        assert_eq!(parent_record.closed_at_seq, Some(EventSeq::new(3)));

        let before_close = SessionId::new("before-close");
        let before_close_record = store
            .create_forked_session(CreateForkedSession {
                source_session_id: parent.clone(),
                session_id: before_close.clone(),
                source_seq: EventSeq::new(2),
                created_at_ms: 20,
            })
            .await
            .expect("fork before close");
        assert_eq!(
            before_close_record.lifecycle_status,
            SessionLifecycleStatus::Open
        );
        assert_eq!(before_close_record.closed_at_seq, None);

        let after_close = SessionId::new("after-close");
        let after_close_record = store
            .create_forked_session(CreateForkedSession {
                source_session_id: parent.clone(),
                session_id: after_close.clone(),
                source_seq: EventSeq::new(3),
                created_at_ms: 21,
            })
            .await
            .expect("fork after close");
        assert_eq!(
            after_close_record.lifecycle_status,
            SessionLifecycleStatus::Closed
        );
        assert_eq!(after_close_record.closed_at_seq, Some(EventSeq::new(3)));

        assert!(matches!(
            store.delete_closed_session(&parent).await,
            Err(SessionStoreError::SessionHasForkChildren { .. })
        ));
        store
            .delete_closed_session(&after_close)
            .await
            .expect("delete closed leaf");
        store
            .append(AppendSessionEvents {
                session_id: before_close.clone(),
                expected_head: before_close_record.head,
                events: vec![lifecycle_closed_event(22)],
            })
            .await
            .expect("close open fork");
        store
            .delete_closed_session(&before_close)
            .await
            .expect("delete second closed leaf");
        store
            .delete_closed_session(&parent)
            .await
            .expect("delete parent after fork leaves");
        assert!(
            store
                .load_session(&parent)
                .await
                .expect("load deleted parent")
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn management_projection_rejects_clones_and_forks() {
        let store = InMemorySessionStore::new();
        let parent = SessionId::new("managed-parent");
        store
            .create_session(CreateSession {
                session_id: parent.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create parent");
        store
            .append(AppendSessionEvents {
                session_id: parent.clone(),
                expected_head: None,
                events: vec![
                    lifecycle_opened_event(10),
                    managed_bindings_event(
                        11,
                        Some(crate::WorkflowEndpointRef {
                            workflow_id: "channels/session-1".to_owned(),
                            workflow_kind: "channelSessionWorkflowV1".to_owned(),
                        }),
                    ),
                ],
            })
            .await
            .expect("admit management");

        let parent_record = store
            .load_session(&parent)
            .await
            .expect("load parent")
            .expect("parent exists");
        assert!(parent_record.managed);

        let clone_error = store
            .create_cloned_session(CreateClonedSession {
                source_session_id: parent.clone(),
                session_id: SessionId::new("managed-clone"),
                created_at_ms: 20,
                opening_events: vec![lifecycle_opened_event(20)],
            })
            .await
            .expect_err("managed session cannot be cloned");
        assert!(matches!(
            clone_error,
            SessionStoreError::ManagedSessionCannotBranch { .. }
        ));

        let fork_error = store
            .create_forked_session(CreateForkedSession {
                source_session_id: parent.clone(),
                session_id: SessionId::new("managed-fork"),
                source_seq: EventSeq::new(2),
                created_at_ms: 21,
            })
            .await
            .expect_err("managed session cannot be forked");
        assert!(matches!(
            fork_error,
            SessionStoreError::ManagedSessionCannotBranch { .. }
        ));

        let tool_only = SessionId::new("tool-only");
        store
            .create_session(CreateSession {
                session_id: tool_only.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 30,
            })
            .await
            .expect("create tool-only session");
        store
            .append(AppendSessionEvents {
                session_id: tool_only.clone(),
                expected_head: None,
                events: vec![lifecycle_opened_event(31), managed_bindings_event(32, None)],
            })
            .await
            .expect("admit tool-only bindings");
        let tool_only = store
            .load_session(&tool_only)
            .await
            .expect("load tool-only session")
            .expect("tool-only session exists");
        assert!(!tool_only.managed);

        let tool_only_fork = store
            .create_forked_session(CreateForkedSession {
                source_session_id: tool_only.session_id,
                session_id: SessionId::new("tool-only-fork"),
                source_seq: EventSeq::new(2),
                created_at_ms: 33,
            })
            .await
            .expect("tool-only session remains forkable");
        assert!(!tool_only_fork.managed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_rejects_expected_head_conflict() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::new("session-a");
        store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create session");
        let first = store
            .append(AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: None,
                events: vec![open_event(10)],
            })
            .await
            .expect("append first");

        let error = store
            .append(AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: None,
                events: vec![open_event(11)],
            })
            .await
            .expect_err("stale append fails");

        assert!(matches!(
            error,
            SessionStoreError::ExpectedHeadMismatch {
                expected: None,
                actual,
                ..
            } if actual == first.head
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_reports_typed_session_errors() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::new("session-a");
        store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create session");

        let duplicate = store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 2,
            })
            .await
            .expect_err("duplicate session fails");
        assert!(matches!(
            duplicate,
            SessionStoreError::SessionAlreadyExists { .. }
        ));

        let missing = store
            .append(AppendSessionEvents {
                session_id: SessionId::new("missing"),
                expected_head: None,
                events: vec![open_event(10)],
            })
            .await
            .expect_err("missing session fails");
        assert!(matches!(missing, SessionStoreError::SessionNotFound { .. }));

        let conflict = store
            .append(AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: Some(SessionPosition {
                    seq: EventSeq::new(1),
                }),
                events: vec![open_event(11)],
            })
            .await
            .expect_err("expected-head conflict fails");
        assert!(matches!(
            conflict,
            SessionStoreError::ExpectedHeadMismatch { .. }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_session_store_clones_with_fresh_log_and_source_lineage() {
        let store = InMemorySessionStore::new();
        let source_id = SessionId::new("source");
        store
            .create_session(CreateSession {
                session_id: source_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        store
            .append(AppendSessionEvents {
                session_id: source_id.clone(),
                expected_head: None,
                events: vec![open_event(10), open_event(11)],
            })
            .await
            .expect("append source");

        let child_id = SessionId::new("clone");
        let child = store
            .create_cloned_session(CreateClonedSession {
                source_session_id: source_id.clone(),
                session_id: child_id.clone(),
                created_at_ms: 20,
                opening_events: vec![test_event(21, "lightspeed.test.clone.opened")],
            })
            .await
            .expect("clone session");

        assert_eq!(child.source_session_id, Some(source_id));
        assert_eq!(child.source_seq, None);
        assert_eq!(
            child.head.as_ref().map(|head| head.seq),
            Some(EventSeq::new(1))
        );
        let page = store
            .read_after(ReadSessionEvents {
                session_id: child_id,
                after: None,
                limit: 10,
            })
            .await
            .expect("read clone");
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].position.seq, EventSeq::new(1));
        assert_eq!(page.entries[0].event.kind, "lightspeed.test.clone.opened");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_fork_reads_stitch_multiple_levels_and_clamp_parent_tail() {
        let store = InMemorySessionStore::new();
        let root = SessionId::new("root");
        store
            .create_session(CreateSession {
                session_id: root.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create root");
        store
            .append(AppendSessionEvents {
                session_id: root.clone(),
                expected_head: None,
                events: vec![
                    test_event(10, "root.1"),
                    test_event(11, "root.2"),
                    test_event(12, "root.3"),
                ],
            })
            .await
            .expect("append root");

        let fork = SessionId::new("fork");
        store
            .create_forked_session(CreateForkedSession {
                source_session_id: root.clone(),
                session_id: fork.clone(),
                source_seq: EventSeq::new(2),
                created_at_ms: 20,
            })
            .await
            .expect("fork root");
        let fork_append = store
            .append(AppendSessionEvents {
                session_id: fork.clone(),
                expected_head: Some(SessionPosition {
                    seq: EventSeq::new(2),
                }),
                events: vec![test_event(21, "fork.3"), test_event(22, "fork.4")],
            })
            .await
            .expect("append fork");
        assert_eq!(
            fork_append
                .entries
                .iter()
                .map(|entry| entry.position.seq)
                .collect::<Vec<_>>(),
            vec![EventSeq::new(3), EventSeq::new(4)]
        );

        store
            .append(AppendSessionEvents {
                session_id: root,
                expected_head: Some(SessionPosition {
                    seq: EventSeq::new(3),
                }),
                events: vec![test_event(30, "root.4-hidden")],
            })
            .await
            .expect("append root tail");

        let grandchild = SessionId::new("grandchild");
        store
            .create_forked_session(CreateForkedSession {
                source_session_id: fork.clone(),
                session_id: grandchild.clone(),
                source_seq: EventSeq::new(3),
                created_at_ms: 40,
            })
            .await
            .expect("fork fork");
        store
            .append(AppendSessionEvents {
                session_id: grandchild.clone(),
                expected_head: Some(SessionPosition {
                    seq: EventSeq::new(3),
                }),
                events: vec![test_event(41, "grandchild.4")],
            })
            .await
            .expect("append grandchild");

        let page = store
            .read_after(ReadSessionEvents {
                session_id: grandchild,
                after: Some(EventSeq::new(1)),
                limit: 10,
            })
            .await
            .expect("read grandchild");
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| (entry.position.seq.as_u64(), entry.event.kind.as_str()))
                .collect::<Vec<_>>(),
            vec![(2, "root.2"), (3, "fork.3"), (4, "grandchild.4")]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_safe_fork_seq_excludes_open_run_and_rejects_inside_run() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::new("session-a");
        store
            .create_session(CreateSession {
                session_id: session_id.clone(),
                display_name: None,
                origin: None,
                created_at_ms: 1,
            })
            .await
            .expect("create session");
        store
            .append(AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: None,
                events: vec![
                    test_event(10, "standalone.1"),
                    run_event(11, "lightspeed.core.run.accepted", 1),
                    run_event(12, "lightspeed.core.run.started", 1),
                ],
            })
            .await
            .expect("append open run");

        assert_eq!(
            store
                .safe_fork_seq(&session_id)
                .await
                .expect("safe fork seq"),
            EventSeq::new(1)
        );
        let error = store
            .create_forked_session(CreateForkedSession {
                source_session_id: session_id.clone(),
                session_id: SessionId::new("bad-fork"),
                source_seq: EventSeq::new(2),
                created_at_ms: 20,
            })
            .await
            .expect_err("fork inside open run fails");
        assert!(matches!(
            error,
            SessionStoreError::InvalidForkPoint {
                source_seq,
                ..
            } if source_seq == EventSeq::new(2)
        ));
    }
}
