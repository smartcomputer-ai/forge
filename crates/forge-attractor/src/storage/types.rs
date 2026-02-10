use forge_turnstore::{BlobHash, ContextId, CorrelationMetadata, StoredTurnEnvelope, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ATTRACTOR_RUN_EVENT_TYPE_ID: &str = "forge.attractor.run_event";
pub const ATTRACTOR_STAGE_EVENT_TYPE_ID: &str = "forge.attractor.stage_event";
pub const ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID: &str = "forge.attractor.checkpoint_event";
pub const ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID: &str = "forge.link.stage_to_agent";
pub const ATTRACTOR_DOT_SOURCE_TYPE_ID: &str = "forge.attractor.dot_source";
pub const ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID: &str = "forge.attractor.graph_snapshot";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttractorCorrelation {
    pub run_id: String,
    pub pipeline_context_id: Option<ContextId>,
    pub node_id: Option<String>,
    pub stage_attempt_id: Option<String>,
    pub parent_turn_id: Option<TurnId>,
    pub sequence_no: u64,
    pub agent_session_id: Option<String>,
    pub agent_context_id: Option<ContextId>,
    pub agent_head_turn_id: Option<TurnId>,
}

impl AttractorCorrelation {
    pub fn to_store_correlation(&self) -> CorrelationMetadata {
        CorrelationMetadata {
            run_id: Some(self.run_id.clone()),
            pipeline_context_id: self.pipeline_context_id.clone(),
            node_id: self.node_id.clone(),
            stage_attempt_id: self.stage_attempt_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            agent_context_id: self.agent_context_id.clone(),
            agent_head_turn_id: self.agent_head_turn_id.clone(),
            parent_turn_id: self.parent_turn_id.clone(),
            sequence_no: Some(self.sequence_no),
            ..CorrelationMetadata::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunEventRecord {
    pub event_kind: String,
    pub timestamp: String,
    pub payload: Value,
    pub correlation: AttractorCorrelation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageEventRecord {
    pub event_kind: String,
    pub timestamp: String,
    pub payload: Value,
    pub correlation: AttractorCorrelation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointEventRecord {
    pub checkpoint_id: String,
    pub timestamp: String,
    pub state_summary: Value,
    pub checkpoint_hash: Option<BlobHash>,
    pub correlation: AttractorCorrelation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageToAgentLinkRecord {
    pub timestamp: String,
    pub run_id: String,
    pub pipeline_context_id: ContextId,
    pub node_id: String,
    pub stage_attempt_id: String,
    pub agent_session_id: String,
    pub agent_context_id: ContextId,
    pub agent_head_turn_id: Option<TurnId>,
    pub parent_turn_id: Option<TurnId>,
    pub sequence_no: u64,
    pub thread_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DotSourceRecord {
    pub timestamp: String,
    pub dot_source: String,
    pub content_hash: BlobHash,
    pub size_bytes: u64,
    pub correlation: AttractorCorrelation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphSnapshotRecord {
    pub timestamp: String,
    pub graph_snapshot: Value,
    pub content_hash: BlobHash,
    pub size_bytes: u64,
    pub correlation: AttractorCorrelation,
}

impl StageToAgentLinkRecord {
    pub fn to_correlation(&self) -> AttractorCorrelation {
        AttractorCorrelation {
            run_id: self.run_id.clone(),
            pipeline_context_id: Some(self.pipeline_context_id.clone()),
            node_id: Some(self.node_id.clone()),
            stage_attempt_id: Some(self.stage_attempt_id.clone()),
            parent_turn_id: self.parent_turn_id.clone(),
            sequence_no: self.sequence_no,
            agent_session_id: Some(self.agent_session_id.clone()),
            agent_context_id: Some(self.agent_context_id.clone()),
            agent_head_turn_id: self.agent_head_turn_id.clone(),
        }
    }
}

pub fn run_event_envelope(record: RunEventRecord) -> StoredTurnEnvelope {
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.correlation.run_id.clone()),
        session_id: None,
        node_id: record.correlation.node_id.clone(),
        stage_attempt_id: record.correlation.stage_attempt_id.clone(),
        event_kind: record.event_kind,
        timestamp: record.timestamp,
        payload: record.payload,
        correlation: record.correlation.to_store_correlation(),
    }
}

pub fn stage_event_envelope(record: StageEventRecord) -> StoredTurnEnvelope {
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.correlation.run_id.clone()),
        session_id: None,
        node_id: record.correlation.node_id.clone(),
        stage_attempt_id: record.correlation.stage_attempt_id.clone(),
        event_kind: record.event_kind,
        timestamp: record.timestamp,
        payload: record.payload,
        correlation: record.correlation.to_store_correlation(),
    }
}

pub fn checkpoint_event_envelope(record: CheckpointEventRecord) -> StoredTurnEnvelope {
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.correlation.run_id.clone()),
        session_id: None,
        node_id: record.correlation.node_id.clone(),
        stage_attempt_id: record.correlation.stage_attempt_id.clone(),
        event_kind: "checkpoint_saved".to_string(),
        timestamp: record.timestamp,
        payload: serde_json::json!({
            "checkpoint_id": record.checkpoint_id,
            "state_summary": record.state_summary,
            "checkpoint_hash": record.checkpoint_hash,
        }),
        correlation: record.correlation.to_store_correlation(),
    }
}

pub fn stage_to_agent_link_envelope(record: StageToAgentLinkRecord) -> StoredTurnEnvelope {
    let correlation = record.to_correlation().to_store_correlation();
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.run_id.clone()),
        session_id: Some(record.agent_session_id.clone()),
        node_id: Some(record.node_id.clone()),
        stage_attempt_id: Some(record.stage_attempt_id.clone()),
        event_kind: "stage_to_agent_link".to_string(),
        timestamp: record.timestamp,
        payload: serde_json::json!({
            "run_id": record.run_id,
            "pipeline_context_id": record.pipeline_context_id,
            "node_id": record.node_id,
            "stage_attempt_id": record.stage_attempt_id,
            "agent_session_id": record.agent_session_id,
            "agent_context_id": record.agent_context_id,
            "agent_head_turn_id": record.agent_head_turn_id,
            "parent_turn_id": record.parent_turn_id,
            "sequence_no": record.sequence_no,
            "thread_key": record.thread_key,
        }),
        correlation,
    }
}

pub fn dot_source_envelope(record: DotSourceRecord) -> StoredTurnEnvelope {
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.correlation.run_id.clone()),
        session_id: None,
        node_id: record.correlation.node_id.clone(),
        stage_attempt_id: record.correlation.stage_attempt_id.clone(),
        event_kind: "dot_source_persisted".to_string(),
        timestamp: record.timestamp,
        payload: serde_json::json!({
            "dot_source": record.dot_source,
            "content_hash": record.content_hash,
            "size_bytes": record.size_bytes,
        }),
        correlation: record.correlation.to_store_correlation(),
    }
}

pub fn graph_snapshot_envelope(record: GraphSnapshotRecord) -> StoredTurnEnvelope {
    StoredTurnEnvelope {
        schema_version: 1,
        run_id: Some(record.correlation.run_id.clone()),
        session_id: None,
        node_id: record.correlation.node_id.clone(),
        stage_attempt_id: record.correlation.stage_attempt_id.clone(),
        event_kind: "graph_snapshot_persisted".to_string(),
        timestamp: record.timestamp,
        payload: serde_json::json!({
            "graph_snapshot": record.graph_snapshot,
            "content_hash": record.content_hash,
            "size_bytes": record.size_bytes,
        }),
        correlation: record.correlation.to_store_correlation(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_to_agent_link_envelope_includes_expected_correlation_fields() {
        let envelope = stage_to_agent_link_envelope(StageToAgentLinkRecord {
            timestamp: "2026-02-10T12:00:00Z".to_string(),
            run_id: "run-1".to_string(),
            pipeline_context_id: "ctx-1".to_string(),
            node_id: "plan".to_string(),
            stage_attempt_id: "attempt-1".to_string(),
            agent_session_id: "session-1".to_string(),
            agent_context_id: "agent-ctx-1".to_string(),
            agent_head_turn_id: Some("42".to_string()),
            parent_turn_id: Some("11".to_string()),
            sequence_no: 7,
            thread_key: Some("thread-main".to_string()),
        });

        assert_eq!(envelope.event_kind, "stage_to_agent_link");
        assert_eq!(
            envelope
                .correlation
                .agent_context_id
                .as_deref()
                .expect("agent context should be present"),
            "agent-ctx-1"
        );
        assert_eq!(envelope.correlation.sequence_no, Some(7));
    }
}
