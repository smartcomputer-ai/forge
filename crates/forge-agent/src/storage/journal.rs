//! Journal storage contract.

use crate::error::ModelError;
use crate::events::AgentEvent;
use crate::ids::{JournalSeq, SessionId};
use crate::journal::{InMemoryJournal, JournalAppendResult};
use async_trait::async_trait;
use std::sync::{Arc, RwLock};

#[async_trait]
pub trait JournalStore: Send + Sync {
    async fn append(&self, event: AgentEvent) -> Result<JournalAppendResult, ModelError>;

    async fn events_after(
        &self,
        after: Option<JournalSeq>,
        limit: usize,
    ) -> Result<Vec<AgentEvent>, ModelError>;

    async fn latest_seq(&self) -> Result<Option<JournalSeq>, ModelError>;
}

#[derive(Clone)]
pub struct InMemoryJournalStore {
    inner: Arc<RwLock<InMemoryJournal>>,
}

impl InMemoryJournalStore {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            inner: Arc::new(RwLock::new(InMemoryJournal::new(session_id))),
        }
    }
}

#[async_trait]
impl JournalStore for InMemoryJournalStore {
    async fn append(&self, event: AgentEvent) -> Result<JournalAppendResult, ModelError> {
        self.inner
            .write()
            .expect("journal store lock poisoned")
            .append(event)
    }

    async fn events_after(
        &self,
        after: Option<JournalSeq>,
        limit: usize,
    ) -> Result<Vec<AgentEvent>, ModelError> {
        let journal = self.inner.read().expect("journal store lock poisoned");
        Ok(journal
            .events()
            .iter()
            .filter(|event| {
                event
                    .journal_seq
                    .is_some_and(|seq| after.is_none_or(|after| seq > after))
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn latest_seq(&self) -> Result<Option<JournalSeq>, ModelError> {
        Ok(self
            .inner
            .read()
            .expect("journal store lock poisoned")
            .latest_seq())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{AgentEvent, AgentEventKind, InputEvent};

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_journal_store_appends_and_reads_after_sequence() {
        let session_id = SessionId::new("session-a");
        let store = InMemoryJournalStore::new(session_id.clone());
        store
            .append(AgentEvent::new(
                "event-1",
                session_id.clone(),
                1,
                AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
            ))
            .await
            .expect("append first");
        store
            .append(AgentEvent::new(
                "event-2",
                session_id.clone(),
                2,
                AgentEventKind::Input(InputEvent::SessionPaused),
            ))
            .await
            .expect("append second");

        let events = store
            .events_after(Some(JournalSeq(1)), 10)
            .await
            .expect("read events");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "event-2");
        assert_eq!(
            store.latest_seq().await.expect("latest seq"),
            Some(JournalSeq(2))
        );
    }
}
