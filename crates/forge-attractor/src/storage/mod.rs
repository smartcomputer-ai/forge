use crate::storage::types::{
    CheckpointEventRecord, DotSourceRecord, GraphSnapshotRecord, RunEventRecord, StageEventRecord,
    StageToAgentLinkRecord, checkpoint_event_envelope, dot_source_envelope,
    graph_snapshot_envelope, run_event_envelope, stage_event_envelope,
    stage_to_agent_link_envelope,
};
use forge_cxdb_runtime::{
    CxdbAppendTurnRequest, CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbRuntimeStore,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CorrelationMetadata {
    pub run_id: Option<String>,
    pub pipeline_context_id: Option<String>,
    pub node_id: Option<String>,
    pub stage_attempt_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub agent_context_id: Option<String>,
    pub agent_head_turn_id: Option<String>,
    pub parent_turn_id: Option<String>,
    pub sequence_no: Option<u64>,
    pub thread_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredTurnEnvelope {
    pub schema_version: u32,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
    pub stage_attempt_id: Option<String>,
    pub event_kind: String,
    pub timestamp: String,
    pub payload: Value,
    pub correlation: CorrelationMetadata,
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

const TAG_SCHEMA_VERSION: u64 = 1;
const TAG_RUN_ID: u64 = 2;
const TAG_SESSION_ID: u64 = 3;
const TAG_NODE_ID: u64 = 4;
const TAG_STAGE_ATTEMPT_ID: u64 = 5;
const TAG_EVENT_KIND: u64 = 6;
const TAG_TIMESTAMP: u64 = 7;
const TAG_PAYLOAD_JSON: u64 = 8;
const TAG_CORR_RUN_ID: u64 = 9;
const TAG_CORR_PIPELINE_CONTEXT_ID: u64 = 10;
const TAG_CORR_NODE_ID: u64 = 11;
const TAG_CORR_STAGE_ATTEMPT_ID: u64 = 12;
const TAG_CORR_AGENT_SESSION_ID: u64 = 13;
const TAG_CORR_AGENT_CONTEXT_ID: u64 = 14;
const TAG_CORR_AGENT_HEAD_TURN_ID: u64 = 15;
const TAG_CORR_PARENT_TURN_ID: u64 = 16;
const TAG_CORR_SEQUENCE_NO: u64 = 17;
const TAG_CORR_THREAD_KEY: u64 = 18;

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

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_stage_event(
        &self,
        context_id: &ContextId,
        record: StageEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError>;

    async fn append_checkpoint_event(
        &self,
        context_id: &ContextId,
        record: CheckpointEventRecord,
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

    async fn append_run_event(
        &self,
        context_id: &ContextId,
        record: RunEventRecord,
        idempotency_key: String,
    ) -> Result<StoredTurn, StorageError> {
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
    ) -> Result<StoredTurn, StorageError> {
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
    ) -> Result<StoredTurn, StorageError> {
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
    ) -> Result<StoredTurn, StorageError> {
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
    ) -> Result<StoredTurn, StorageError> {
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
    ) -> Result<StoredTurn, StorageError> {
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
}

async fn append_record_runtime<B, H>(
    store: &CxdbRuntimeStore<B, H>,
    context_id: &ContextId,
    type_id: &str,
    envelope: StoredTurnEnvelope,
    idempotency_key: String,
) -> Result<StoredTurn, StorageError>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    let payload = encode_stored_turn_envelope(&envelope)?;
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

pub(crate) fn decode_stored_turn_envelope(
    payload: &[u8],
) -> Result<StoredTurnEnvelope, StorageError> {
    if let Ok(projected) = serde_json::from_slice::<Value>(payload) {
        if let Some(envelope) = decode_projection_envelope(&projected)? {
            return Ok(envelope);
        }
    }

    let tagged: BTreeMap<u64, Value> = rmp_serde::from_slice(payload)
        .map_err(|err| StorageError::Serialization(format!("msgpack decode failed: {err}")))?;
    envelope_from_tagged_map(&tagged)
}

pub(crate) fn encode_stored_turn_envelope(
    envelope: &StoredTurnEnvelope,
) -> Result<Vec<u8>, StorageError> {
    let payload_json = serde_json::to_string(&envelope.payload)
        .map_err(|err| StorageError::Serialization(err.to_string()))?;
    let mut tagged: BTreeMap<u64, Value> = BTreeMap::new();
    tagged.insert(
        TAG_SCHEMA_VERSION,
        Value::Number((envelope.schema_version as u64).into()),
    );
    tagged.insert(
        TAG_RUN_ID,
        optional_string_value(envelope.run_id.as_deref()),
    );
    tagged.insert(
        TAG_SESSION_ID,
        optional_string_value(envelope.session_id.as_deref()),
    );
    tagged.insert(
        TAG_NODE_ID,
        optional_string_value(envelope.node_id.as_deref()),
    );
    tagged.insert(
        TAG_STAGE_ATTEMPT_ID,
        optional_string_value(envelope.stage_attempt_id.as_deref()),
    );
    tagged.insert(TAG_EVENT_KIND, Value::String(envelope.event_kind.clone()));
    tagged.insert(TAG_TIMESTAMP, Value::String(envelope.timestamp.clone()));
    tagged.insert(TAG_PAYLOAD_JSON, Value::String(payload_json));
    tagged.insert(
        TAG_CORR_RUN_ID,
        optional_string_value(envelope.correlation.run_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_PIPELINE_CONTEXT_ID,
        optional_string_value(envelope.correlation.pipeline_context_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_NODE_ID,
        optional_string_value(envelope.correlation.node_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_STAGE_ATTEMPT_ID,
        optional_string_value(envelope.correlation.stage_attempt_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_AGENT_SESSION_ID,
        optional_string_value(envelope.correlation.agent_session_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_AGENT_CONTEXT_ID,
        optional_string_value(envelope.correlation.agent_context_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_AGENT_HEAD_TURN_ID,
        optional_string_value(envelope.correlation.agent_head_turn_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_PARENT_TURN_ID,
        optional_string_value(envelope.correlation.parent_turn_id.as_deref()),
    );
    tagged.insert(
        TAG_CORR_SEQUENCE_NO,
        envelope
            .correlation
            .sequence_no
            .map(|value| Value::Number(value.into()))
            .unwrap_or(Value::Null),
    );
    tagged.insert(
        TAG_CORR_THREAD_KEY,
        optional_string_value(envelope.correlation.thread_key.as_deref()),
    );
    rmp_serde::to_vec_named(&tagged)
        .map_err(|err| StorageError::Serialization(format!("msgpack encode failed: {err}")))
}

fn decode_projection_envelope(
    projected: &Value,
) -> Result<Option<StoredTurnEnvelope>, StorageError> {
    let Some(object) = projected.as_object() else {
        return Ok(None);
    };
    if !object.contains_key("schema_version") || !object.contains_key("payload_json") {
        return Ok(None);
    }

    let payload = parse_payload_json_field(object.get("payload_json"))?;
    Ok(Some(StoredTurnEnvelope {
        schema_version: object
            .get("schema_version")
            .and_then(Value::as_u64)
            .map(|value| value as u32)
            .unwrap_or(1),
        run_id: optional_string_field(object.get("run_id")),
        session_id: optional_string_field(object.get("session_id")),
        node_id: optional_string_field(object.get("node_id")),
        stage_attempt_id: optional_string_field(object.get("stage_attempt_id")),
        event_kind: object
            .get("event_kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        timestamp: object
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        payload,
        correlation: CorrelationMetadata {
            run_id: optional_string_field(object.get("corr_run_id")),
            pipeline_context_id: optional_string_field(object.get("corr_pipeline_context_id")),
            node_id: optional_string_field(object.get("corr_node_id")),
            stage_attempt_id: optional_string_field(object.get("corr_stage_attempt_id")),
            agent_session_id: optional_string_field(object.get("corr_agent_session_id")),
            agent_context_id: optional_string_field(object.get("corr_agent_context_id")),
            agent_head_turn_id: optional_string_field(object.get("corr_agent_head_turn_id")),
            parent_turn_id: optional_string_field(object.get("corr_parent_turn_id")),
            sequence_no: object.get("corr_sequence_no").and_then(Value::as_u64),
            thread_key: optional_string_field(object.get("corr_thread_key")),
        },
    }))
}

fn envelope_from_tagged_map(
    tagged: &BTreeMap<u64, Value>,
) -> Result<StoredTurnEnvelope, StorageError> {
    Ok(StoredTurnEnvelope {
        schema_version: tagged
            .get(&TAG_SCHEMA_VERSION)
            .and_then(Value::as_u64)
            .map(|value| value as u32)
            .unwrap_or(1),
        run_id: optional_string_field(tagged.get(&TAG_RUN_ID)),
        session_id: optional_string_field(tagged.get(&TAG_SESSION_ID)),
        node_id: optional_string_field(tagged.get(&TAG_NODE_ID)),
        stage_attempt_id: optional_string_field(tagged.get(&TAG_STAGE_ATTEMPT_ID)),
        event_kind: tagged
            .get(&TAG_EVENT_KIND)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        timestamp: tagged
            .get(&TAG_TIMESTAMP)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        payload: parse_payload_json_field(tagged.get(&TAG_PAYLOAD_JSON))?,
        correlation: CorrelationMetadata {
            run_id: optional_string_field(tagged.get(&TAG_CORR_RUN_ID)),
            pipeline_context_id: optional_string_field(tagged.get(&TAG_CORR_PIPELINE_CONTEXT_ID)),
            node_id: optional_string_field(tagged.get(&TAG_CORR_NODE_ID)),
            stage_attempt_id: optional_string_field(tagged.get(&TAG_CORR_STAGE_ATTEMPT_ID)),
            agent_session_id: optional_string_field(tagged.get(&TAG_CORR_AGENT_SESSION_ID)),
            agent_context_id: optional_string_field(tagged.get(&TAG_CORR_AGENT_CONTEXT_ID)),
            agent_head_turn_id: optional_string_field(tagged.get(&TAG_CORR_AGENT_HEAD_TURN_ID)),
            parent_turn_id: optional_string_field(tagged.get(&TAG_CORR_PARENT_TURN_ID)),
            sequence_no: tagged.get(&TAG_CORR_SEQUENCE_NO).and_then(Value::as_u64),
            thread_key: optional_string_field(tagged.get(&TAG_CORR_THREAD_KEY)),
        },
    })
}

fn parse_payload_json_field(value: Option<&Value>) -> Result<Value, StorageError> {
    let Some(payload_json) = value.and_then(Value::as_str) else {
        return Ok(Value::Null);
    };
    serde_json::from_str(payload_json)
        .map_err(|err| StorageError::Serialization(format!("payload_json decode failed: {err}")))
}

fn optional_string_value(value: Option<&str>) -> Value {
    value
        .map(|inner| Value::String(inner.to_string()))
        .unwrap_or(Value::Null)
}

fn optional_string_field(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(std::string::ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_msgpack_roundtrip_preserves_payload_and_correlation() {
        let envelope = StoredTurnEnvelope {
            schema_version: 1,
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            node_id: Some("plan".to_string()),
            stage_attempt_id: Some("plan:attempt:1".to_string()),
            event_kind: "stage_completed".to_string(),
            timestamp: "123.456Z".to_string(),
            payload: json!({"status":"success","attempt":1}),
            correlation: CorrelationMetadata {
                run_id: Some("run-1".to_string()),
                pipeline_context_id: Some("ctx-1".to_string()),
                node_id: Some("plan".to_string()),
                stage_attempt_id: Some("plan:attempt:1".to_string()),
                agent_session_id: Some("agent-session-1".to_string()),
                agent_context_id: Some("agent-ctx-1".to_string()),
                agent_head_turn_id: Some("42".to_string()),
                parent_turn_id: Some("7".to_string()),
                sequence_no: Some(9),
                thread_key: Some("main".to_string()),
            },
        };

        let bytes = encode_stored_turn_envelope(&envelope).expect("encode should succeed");
        let decoded = decode_stored_turn_envelope(&bytes).expect("decode should succeed");
        assert_eq!(decoded, envelope);
    }
}
