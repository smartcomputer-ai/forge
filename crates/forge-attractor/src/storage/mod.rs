use crate::storage::types::{
    CheckpointEventRecord, DotSourceRecord, GraphSnapshotRecord, RunEventRecord, StageEventRecord,
    StageToAgentLinkRecord, checkpoint_event_envelope, dot_source_envelope,
    graph_snapshot_envelope, run_event_envelope, stage_event_envelope,
    stage_to_agent_link_envelope,
};
use forge_turnstore::{
    AppendTurnRequest, ContextId, StoreContext, StoredTurn, StoredTurnRef, TurnId, TurnStore,
    TurnStoreError,
};
use std::sync::Arc;

pub mod types;

pub use types::{
    ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID, ATTRACTOR_DOT_SOURCE_TYPE_ID,
    ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID, ATTRACTOR_RUN_EVENT_TYPE_ID, ATTRACTOR_STAGE_EVENT_TYPE_ID,
    ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID, AttractorCorrelation,
    CheckpointEventRecord as AttractorCheckpointEventRecord,
    DotSourceRecord as AttractorDotSourceRecord,
    GraphSnapshotRecord as AttractorGraphSnapshotRecord, RunEventRecord as AttractorRunEventRecord,
    StageEventRecord as AttractorStageEventRecord,
    StageToAgentLinkRecord as AttractorStageToAgentLinkRecord,
};

pub type SharedAttractorStorageWriter = Arc<dyn AttractorStorageWriter>;
pub type SharedAttractorStorageReader = Arc<dyn AttractorStorageReader>;

#[async_trait::async_trait]
pub trait AttractorStorageWriter: Send + Sync {
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError>;

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError>;
}

#[async_trait::async_trait]
pub trait AttractorStorageReader: Send + Sync {
    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, TurnStoreError>;

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> Result<Vec<StoredTurn>, TurnStoreError>;
}

#[async_trait::async_trait]
impl<T> AttractorStorageWriter for T
where
    T: TurnStore + Send + Sync,
{
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        self.create_context(base_turn_id).await
    }

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&run_event_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_RUN_EVENT_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&stage_event_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_STAGE_EVENT_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&checkpoint_event_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&stage_to_agent_link_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&dot_source_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_DOT_SOURCE_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        let payload = serde_json::to_vec(&graph_snapshot_envelope(record))
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        self.append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }
}

#[async_trait::async_trait]
impl<T> AttractorStorageReader for T
where
    T: TurnStore + Send + Sync,
{
    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, TurnStoreError> {
        TurnStore::get_head(self, context_id).await
    }

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> Result<Vec<StoredTurn>, TurnStoreError> {
        TurnStore::list_turns(self, context_id, before_turn_id, limit).await
    }
}
