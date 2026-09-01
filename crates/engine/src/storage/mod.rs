//! Logical storage contracts for agent runners.
//!
//! These traits describe what the agent runtime needs without choosing a
//! production backend. Local runners can use the in-memory implementations,
//! while Postgres/Temporal runners adapt these contracts to their own
//! durability model.

pub mod blobs;
pub mod session;

pub use crate::session::{
    SessionEntry, StoredSessionEntry, UncommittedSessionEvent, UncommittedStoredEvent,
};
pub use blobs::{
    BlobCacheLimits, BlobCacheStats, BlobEdge, BlobGraphStore, BlobInfo, BlobStore, BlobStoreError,
    CachedBlobStore, InMemoryBlobCache, InMemoryBlobStore, SessionBlobRoot, ensure_engine_blobs,
};
pub use session::{
    AdvanceSessionCheckpoint, AppendSessionEvents, AppendSessionEventsResult, CreateClonedSession,
    CreateForkedSession, CreateSession, InMemorySessionStore, ListSessions, ReadSessionEventRange,
    ReadSessionEvents, SessionCheckpoint, SessionLifecycleStatus, SessionListCursor,
    SessionListPage, SessionOrigin, SessionOriginCounts, SessionOriginKind, SessionOriginLimit,
    SessionPage, SessionRecord, SessionStore, SessionStoreError, apply_lifecycle_projection,
    check_origin_limits, is_terminal_run_entry, largest_safe_fork_seq,
    largest_safe_fork_seq_from_state, lifecycle_at_fork, validate_fork_point,
};
