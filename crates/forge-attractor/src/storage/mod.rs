use crate::storage::types::{
    ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID, ATTRACTOR_RUN_EVENT_TYPE_ID, ATTRACTOR_STAGE_EVENT_TYPE_ID,
    ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID, CheckpointEventRecord, RunEventRecord, StageEventRecord,
    StageToAgentLinkRecord, checkpoint_event_envelope, run_event_envelope, stage_event_envelope,
    stage_to_agent_link_envelope,
};
use forge_turnstore::{
    AppendTurnRequest, ContextId, StoreContext, StoredTurn, TurnId, TurnStore, TurnStoreError,
};
use std::sync::Arc;

pub mod types;

pub use types::{
    AttractorCorrelation, CheckpointEventRecord as AttractorCheckpointEventRecord,
    RunEventRecord as AttractorRunEventRecord, StageEventRecord as AttractorStageEventRecord,
    StageToAgentLinkRecord as AttractorStageToAgentLinkRecord,
};

pub type SharedAttractorStorageWriter = Arc<dyn AttractorStorageWriter>;

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
            type_id: ATTRACTOR_RUN_EVENT_TYPE_ID.to_string(),
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
            type_id: ATTRACTOR_STAGE_EVENT_TYPE_ID.to_string(),
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
            type_id: ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID.to_string(),
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
            type_id: ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
    }
}
