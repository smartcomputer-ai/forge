//! Bounded session-state snapshot storage contract.

use crate::ids::{JournalSeq, SessionId};
use crate::state::SessionState;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SnapshotStoreError {
    #[error("snapshot store failure: {message}")]
    Store { message: String },

    #[error("snapshot session mismatch: key {key}, state {state}")]
    SessionMismatch { key: String, state: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub session_id: SessionId,
    pub latest_journal_seq: Option<JournalSeq>,
    pub state: SessionState,
    pub saved_at_ms: u64,
    pub metadata: BTreeMap<String, String>,
}

impl StateSnapshot {
    pub fn new(state: SessionState, saved_at_ms: u64) -> Self {
        Self {
            session_id: state.session_id.clone(),
            latest_journal_seq: state.latest_journal_seq,
            state,
            saved_at_ms,
            metadata: BTreeMap::new(),
        }
    }
}

#[async_trait]
pub trait SnapshotStore: Send + Sync {
    async fn save_latest(&self, snapshot: StateSnapshot) -> Result<(), SnapshotStoreError>;

    async fn load_latest(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<StateSnapshot>, SnapshotStoreError>;
}

#[derive(Clone, Default)]
pub struct InMemorySnapshotStore {
    snapshots: Arc<RwLock<BTreeMap<SessionId, StateSnapshot>>>,
}

impl InMemorySnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SnapshotStore for InMemorySnapshotStore {
    async fn save_latest(&self, snapshot: StateSnapshot) -> Result<(), SnapshotStoreError> {
        if snapshot.session_id != snapshot.state.session_id {
            return Err(SnapshotStoreError::SessionMismatch {
                key: snapshot.session_id.to_string(),
                state: snapshot.state.session_id.to_string(),
            });
        }
        self.snapshots
            .write()
            .map_err(|_| SnapshotStoreError::Store {
                message: "snapshot write lock poisoned".into(),
            })?
            .insert(snapshot.session_id.clone(), snapshot);
        Ok(())
    }

    async fn load_latest(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<StateSnapshot>, SnapshotStoreError> {
        self.snapshots
            .read()
            .map_err(|_| SnapshotStoreError::Store {
                message: "snapshot read lock poisoned".into(),
            })
            .map(|snapshots| snapshots.get(session_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_snapshot_store_saves_and_loads_latest_state() {
        let session_id = SessionId::new("session-a");
        let mut state = SessionState::new(session_id.clone(), Default::default(), 10);
        state.latest_journal_seq = Some(JournalSeq(7));
        let snapshot = StateSnapshot::new(state, 12);
        let store = InMemorySnapshotStore::new();

        store.save_latest(snapshot).await.expect("save snapshot");
        let loaded = store
            .load_latest(&session_id)
            .await
            .expect("load snapshot")
            .expect("snapshot exists");

        assert_eq!(loaded.latest_journal_seq, Some(JournalSeq(7)));
        assert_eq!(loaded.saved_at_ms, 12);
    }
}
