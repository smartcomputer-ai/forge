use crate::storage::types::{
    CheckpointSavedRecord, DotSourceRecord, GraphSnapshotRecord, InterviewLifecycleRecord,
    ParallelLifecycleRecord, RouteDecisionRecord, RunLifecycleRecord, StageLifecycleRecord,
    StageToAgentLinkRecord,
};
use forge_cxdb_runtime::{
    CxdbAppendTurnRequest, CxdbBinaryClient, CxdbClientError, CxdbFsSnapshotCapture,
    CxdbFsSnapshotPolicy, CxdbHttpClient, CxdbRuntimeStore,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

pub mod types;

pub use types::{
    ATTRACTOR_CHECKPOINT_SAVED_TYPE_ID, ATTRACTOR_DOT_SOURCE_TYPE_ID,
    ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID, ATTRACTOR_INTERVIEW_LIFECYCLE_TYPE_ID,
    ATTRACTOR_PARALLEL_LIFECYCLE_TYPE_ID, ATTRACTOR_ROUTE_DECISION_TYPE_ID,
    ATTRACTOR_RUN_LIFECYCLE_TYPE_ID, ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID,
    ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
    CheckpointSavedRecord as AttractorCheckpointSavedRecord,
    DotSourceRecord as AttractorDotSourceRecord, FsSnapshotStats as AttractorFsSnapshotStats,
    GraphSnapshotRecord as AttractorGraphSnapshotRecord,
    InterviewLifecycleRecord as AttractorInterviewLifecycleRecord,
    ParallelLifecycleRecord as AttractorParallelLifecycleRecord,
    RouteDecisionRecord as AttractorRouteDecisionRecord,
    RunLifecycleRecord as AttractorRunLifecycleRecord,
    StageLifecycleRecord as AttractorStageLifecycleRecord,
    StageToAgentLinkRecord as AttractorStageToAgentLinkRecord,
};

pub type ContextId = String;
pub type TurnId = String;
pub type BlobHash = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreContext {
    pub context_id: ContextId,
    pub head_turn_id: TurnId,
    pub head_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTurnRef {
    pub context_id: ContextId,
    pub turn_id: TurnId,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendTurnRequest {
    pub context_id: ContextId,
    pub parent_turn_id: Option<TurnId>,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTurn {
    pub context_id: ContextId,
    pub turn_id: TurnId,
    pub parent_turn_id: TurnId,
    pub depth: u32,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: Option<String>,
    pub content_hash: Option<BlobHash>,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("resource not found: {resource} ({id})")]
    NotFound { resource: &'static str, id: String },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("backend failure: {0}")]
    Backend(String),
}

pub type SharedAttractorStorageWriter = Arc<dyn AttractorStorageWriter>;
pub type SharedAttractorStorageReader = Arc<dyn AttractorStorageReader>;

fn encode_part(part: &str) -> String {
    format!("{}:{}", part.len(), part)
}

pub fn attractor_idempotency_key(
    run_id: &str,
    node_id: &str,
    stage_attempt_id: &str,
    event_kind: &str,
    sequence_no: u64,
) -> String {
    format!(
        "forge-attractor:v1|{}|{}|{}|{}|{}",
        encode_part(run_id),
        encode_part(node_id),
        encode_part(stage_attempt_id),
        encode_part(event_kind),
        sequence_no
    )
}

#[async_trait::async_trait]
pub trait AttractorStorageWriter: Send + Sync {
    async fn create_run_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, StorageError>;

    async fn append_run_lifecycle(
        &self,
        context_id: &ContextId,
        record: RunLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_stage_lifecycle(
        &self,
        context_id: &ContextId,
        record: StageLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_parallel_lifecycle(
        &self,
        context_id: &ContextId,
        record: ParallelLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_interview_lifecycle(
        &self,
        context_id: &ContextId,
        record: InterviewLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_checkpoint_saved(
        &self,
        context_id: &ContextId,
        record: CheckpointSavedRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_route_decision(
        &self,
        context_id: &ContextId,
        record: RouteDecisionRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;
}

#[async_trait::async_trait]
pub trait AttractorStorageReader: Send + Sync {
    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, StorageError>;

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> Result<Vec<StoredTurn>, StorageError>;
}

#[async_trait::async_trait]
pub trait AttractorArtifactWriter: Send + Sync {
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, StorageError>;

    async fn capture_upload_workspace(
        &self,
        workspace_root: &Path,
        policy: &CxdbFsSnapshotPolicy,
    ) -> Result<CxdbFsSnapshotCapture, StorageError> {
        let _ = (workspace_root, policy);
        Err(StorageError::Unsupported(
            "capture_upload_workspace is not supported by this artifact writer".to_string(),
        ))
    }

    async fn attach_fs(
        &self,
        turn_id: &TurnId,
        fs_root_hash: &BlobHash,
    ) -> Result<(), StorageError> {
        let _ = (turn_id, fs_root_hash);
        Err(StorageError::Unsupported(
            "attach_fs is not supported by this artifact writer".to_string(),
        ))
    }
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
    ) -> Result<StoreContext, StorageError> {
        let context = self
            .create_context(base_turn_id)
            .await
            .map_err(cxdb_error_to_storage)?;
        Ok(StoreContext {
            context_id: context.context_id,
            head_turn_id: context.head_turn_id,
            head_depth: context.head_depth,
        })
    }

    async fn append_run_lifecycle(
        &self,
        context_id: &ContextId,
        record: RunLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_RUN_LIFECYCLE_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_stage_lifecycle(
        &self,
        context_id: &ContextId,
        record: StageLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_parallel_lifecycle(
        &self,
        context_id: &ContextId,
        record: ParallelLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_PARALLEL_LIFECYCLE_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_interview_lifecycle(
        &self,
        context_id: &ContextId,
        record: InterviewLifecycleRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_INTERVIEW_LIFECYCLE_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_checkpoint_saved(
        &self,
        context_id: &ContextId,
        record: CheckpointSavedRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_CHECKPOINT_SAVED_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_route_decision(
        &self,
        context_id: &ContextId,
        record: RouteDecisionRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_ROUTE_DECISION_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_stage_to_agent_link(
        &self,
        context_id: &ContextId,
        record: StageToAgentLinkRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_dot_source(
        &self,
        context_id: &ContextId,
        record: DotSourceRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_DOT_SOURCE_TYPE_ID,
            record,
            idempotency_key,
        )
        .await
    }

    async fn append_graph_snapshot(
        &self,
        context_id: &ContextId,
        record: GraphSnapshotRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
        append_record_runtime(
            self,
            context_id,
            types::ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID,
            record,
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
    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, StorageError> {
        let head = self
            .get_head(context_id)
            .await
            .map_err(cxdb_error_to_storage)?;
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
    ) -> Result<Vec<StoredTurn>, StorageError> {
        let turns = self
            .list_turns(context_id, before_turn_id, limit)
            .await
            .map_err(cxdb_error_to_storage)?;
        Ok(turns.into_iter().map(runtime_to_stored_turn).collect())
    }
}

#[async_trait::async_trait]
impl<B, H> AttractorArtifactWriter for CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, StorageError> {
        self.put_blob(raw_bytes)
            .await
            .map_err(cxdb_error_to_storage)
    }

    async fn capture_upload_workspace(
        &self,
        workspace_root: &Path,
        policy: &CxdbFsSnapshotPolicy,
    ) -> Result<CxdbFsSnapshotCapture, StorageError> {
        CxdbRuntimeStore::capture_upload_workspace(self, workspace_root, policy)
            .await
            .map_err(cxdb_error_to_storage)
    }

    async fn attach_fs(
        &self,
        turn_id: &TurnId,
        fs_root_hash: &BlobHash,
    ) -> Result<(), StorageError> {
        CxdbRuntimeStore::attach_fs(self, turn_id, fs_root_hash)
            .await
            .map_err(cxdb_error_to_storage)
    }
}

async fn append_record_runtime<B, H, R>(
    store: &CxdbRuntimeStore<B, H>,
    context_id: &ContextId,
    type_id: &str,
    record: R,
    idempotency_key: String,
) -> Result<StoredTurn, StorageError>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
    R: Serialize,
{
    let payload = encode_typed_record(&record)?;
    let turn = store
        .append_turn(CxdbAppendTurnRequest {
            context_id: context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload,
            idempotency_key,
            fs_root_hash: None,
        })
        .await
        .map_err(cxdb_error_to_storage)?;
    Ok(runtime_to_stored_turn(turn))
}

fn runtime_to_stored_turn(turn: forge_cxdb_runtime::CxdbStoredTurn) -> StoredTurn {
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

fn cxdb_error_to_storage(error: CxdbClientError) -> StorageError {
    match error {
        CxdbClientError::NotFound { resource, id } => StorageError::NotFound { resource, id },
        CxdbClientError::Conflict(message) => StorageError::Conflict(message),
        CxdbClientError::InvalidInput(message) => StorageError::InvalidInput(message),
        CxdbClientError::Backend(message) => StorageError::Backend(message),
    }
}

pub(crate) fn encode_typed_record<T: Serialize>(record: &T) -> Result<Vec<u8>, StorageError> {
    rmp_serde::to_vec_named(record)
        .map_err(|err| StorageError::Serialization(format!("msgpack encode failed: {err}")))
}

pub(crate) fn decode_typed_record<T: DeserializeOwned>(payload: &[u8]) -> Result<T, StorageError> {
    if let Ok(projected) = serde_json::from_slice::<T>(payload) {
        return Ok(projected);
    }
    rmp_serde::from_slice(payload)
        .map_err(|err| StorageError::Serialization(format!("msgpack decode failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestRecord {
        kind: String,
        sequence_no: u64,
        detail: String,
    }

    #[test]
    fn typed_record_msgpack_roundtrip_preserves_fields() {
        let record = TestRecord {
            kind: "started".to_string(),
            sequence_no: 7,
            detail: "ok".to_string(),
        };

        let bytes = encode_typed_record(&record).expect("encode should succeed");
        let decoded: TestRecord = decode_typed_record(&bytes).expect("decode should succeed");
        assert_eq!(decoded, record);
    }
}
