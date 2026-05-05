//! Logical storage contracts for agent runners.
//!
//! These traits describe what the agent runtime needs without choosing a
//! production backend. Local runners can use the in-memory implementations,
//! while CXDB/Postgres/Temporal runners adapt these contracts to their own
//! durability model.

pub mod agents;
pub mod artifacts;
pub mod journal;
pub mod snapshots;

pub use agents::{AgentDefinitionStore, AgentDefinitionStoreError, InMemoryAgentDefinitionStore};
pub use artifacts::{ArtifactStore, ArtifactStoreError, ArtifactWrite, InMemoryArtifactStore};
pub use journal::{InMemoryJournalStore, JournalStore};
pub use snapshots::{InMemorySnapshotStore, SnapshotStore, SnapshotStoreError, StateSnapshot};
