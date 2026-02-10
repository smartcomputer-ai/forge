use crate::storage::types::{
    CheckpointEventRecord, DotSourceRecord, GraphSnapshotRecord, RunEventRecord, StageEventRecord,
    StageToAgentLinkRecord, checkpoint_event_envelope, dot_source_envelope,
    graph_snapshot_envelope, run_event_envelope, stage_event_envelope,
    stage_to_agent_link_envelope,
};
use forge_turnstore::{
    AppendTurnRequest, ArtifactStore, FsTurnStore, MemoryTurnStore, TurnStore, TurnStoreError,
};
pub use forge_turnstore::{BlobHash, ContextId, StoreContext, StoredTurn, StoredTurnRef, TurnId};
use forge_turnstore_cxdb::{
    CxdbAppendTurnRequest, CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbRuntimeStore,
    CxdbTurnStore,
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
pub trait AttractorArtifactWriter: Send + Sync {
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, TurnStoreError>;
}

#[async_trait::async_trait]
impl<B, H> AttractorStorageWriter for CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        let context = self
            .create_context(base_turn_id)
            .await
            .map_err(cxdb_error_to_turnstore)?;
        Ok(StoreContext {
            context_id: context.context_id,
            head_turn_id: context.head_turn_id,
            head_depth: context.head_depth,
        })
    }

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_RUN_EVENT_TYPE_ID,
            run_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_STAGE_EVENT_TYPE_ID,
            stage_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID,
            checkpoint_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
            stage_to_agent_link_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_DOT_SOURCE_TYPE_ID,
            dot_source_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID,
            graph_snapshot_envelope(record),
            idempotency_key,
        )
        .await
    }
}

#[async_trait::async_trait]
impl<B, H> AttractorStorageReader for CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, TurnStoreError> {
        let head = self
            .get_head(context_id)
            .await
            .map_err(cxdb_error_to_turnstore)?;
        Ok(StoredTurnRef {
            context_id: head.context_id,
            turn_id: head.turn_id,
            depth: head.depth,
        })
    }

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> Result<Vec<StoredTurn>, TurnStoreError> {
        let turns = self
            .list_turns(context_id, before_turn_id, limit)
            .await
            .map_err(cxdb_error_to_turnstore)?;
        Ok(turns.into_iter().map(runtime_to_stored_turn).collect())
    }
}

#[async_trait::async_trait]
impl<B, H> AttractorArtifactWriter for CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, TurnStoreError> {
        self.put_blob(raw_bytes)
            .await
            .map_err(cxdb_error_to_turnstore)
    }
}

#[async_trait::async_trait]
impl AttractorStorageWriter for MemoryTurnStore {
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        TurnStore::create_context(self, base_turn_id).await
    }

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_RUN_EVENT_TYPE_ID,
            run_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_EVENT_TYPE_ID,
            stage_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID,
            checkpoint_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
            stage_to_agent_link_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_DOT_SOURCE_TYPE_ID,
            dot_source_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID,
            graph_snapshot_envelope(record),
            idempotency_key,
        )
        .await
    }
}

#[async_trait::async_trait]
impl AttractorStorageReader for MemoryTurnStore {
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

#[async_trait::async_trait]
impl AttractorStorageWriter for FsTurnStore {
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        TurnStore::create_context(self, base_turn_id).await
    }

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_RUN_EVENT_TYPE_ID,
            run_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_EVENT_TYPE_ID,
            stage_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID,
            checkpoint_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
            stage_to_agent_link_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_DOT_SOURCE_TYPE_ID,
            dot_source_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID,
            graph_snapshot_envelope(record),
            idempotency_key,
        )
        .await
    }
}

#[async_trait::async_trait]
impl AttractorStorageReader for FsTurnStore {
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

#[async_trait::async_trait]
impl<B, H> AttractorStorageWriter for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        TurnStore::create_context(self, base_turn_id).await
    }

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_RUN_EVENT_TYPE_ID,
            run_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_EVENT_TYPE_ID,
            stage_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID,
            checkpoint_event_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
            stage_to_agent_link_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_DOT_SOURCE_TYPE_ID,
            dot_source_envelope(record),
            idempotency_key,
        )
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        append_record_turnstore(
            self,
            context_id,
            types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID,
            graph_snapshot_envelope(record),
            idempotency_key,
        )
        .await
    }
}

#[async_trait::async_trait]
impl<B, H> AttractorStorageReader for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
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

#[async_trait::async_trait]
impl<B, H> AttractorArtifactWriter for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, TurnStoreError> {
        ArtifactStore::put_blob(self, raw_bytes).await
    }
}

async fn append_record_runtime<B, H>(
    store: &CxdbRuntimeStore<B, H>,
    context_id: &ContextId,
    type_id: &str,
    envelope: forge_turnstore::StoredTurnEnvelope,
    idempotency_key: String,
) -> Result<StoredTurn, TurnStoreError>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    let payload = serde_json::to_vec(&envelope)
        .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
    let turn = store
        .append_turn(CxdbAppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
        .map_err(cxdb_error_to_turnstore)?;
    Ok(runtime_to_stored_turn(turn))
}

async fn append_record_turnstore<T: TurnStore + Send + Sync>(
    store: &T,
    context_id: &ContextId,
    type_id: &str,
    envelope: forge_turnstore::StoredTurnEnvelope,
    idempotency_key: String,
) -> Result<StoredTurn, TurnStoreError> {
    let payload = serde_json::to_vec(&envelope)
        .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
    store
        .append_turn(AppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
        })
        .await
}

fn runtime_to_stored_turn(turn: forge_turnstore_cxdb::CxdbStoredTurn) -> StoredTurn {
    StoredTurn {
        context_id: turn.context_id,
        turn_id: turn.turn_id,
        parent_turn_id: turn.parent_turn_id,
        depth: turn.depth,
        type_id: turn.type_id,
        type_version: turn.type_version,
        payload: turn.payload,
        idempotency_key: turn.idempotency_key,
        content_hash: turn.content_hash,
    }
}

fn cxdb_error_to_turnstore(error: CxdbClientError) -> TurnStoreError {
    match error {
        CxdbClientError::NotFound { resource, id } => TurnStoreError::NotFound { resource, id },
        CxdbClientError::Conflict(message) => TurnStoreError::Conflict(message),
        CxdbClientError::InvalidInput(message) => TurnStoreError::InvalidInput(message),
        CxdbClientError::Backend(message) => TurnStoreError::Backend(message),
    }
}
