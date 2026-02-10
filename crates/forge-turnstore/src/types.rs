use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryBundle {
    pub bundle_id: String,
    pub bundle_json: Vec<u8>,
}

fn encode_part(part: &str) -> String {
    format!("{}:{}", part.len(), part)
}

pub fn agent_idempotency_key(session_id: &str, local_turn_index: u64, event_kind: &str) -> String {
    format!(
        "forge-agent:v1|{}|{}|{}",
        encode_part(session_id),
        local_turn_index,
        encode_part(event_kind)
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_idempotency_key_same_inputs_expected_stable_output() {
        let key_a = agent_idempotency_key("session-1", 7, "assistant_completion");
        let key_b = agent_idempotency_key("session-1", 7, "assistant_completion");

        assert_eq!(key_a, key_b);
        assert_eq!(
            key_a,
            "forge-agent:v1|9:session-1|7|20:assistant_completion"
        );
    }

    #[test]
    fn attractor_idempotency_key_distinct_sequence_expected_distinct_keys() {
        let first = attractor_idempotency_key("run-1", "node-A", "attempt-2", "stage_started", 1);
        let second = attractor_idempotency_key("run-1", "node-A", "attempt-2", "stage_started", 2);

        assert_ne!(first, second);
    }

    #[test]
    fn stored_turn_envelope_round_trip_expected_lossless() {
        let envelope = StoredTurnEnvelope {
            schema_version: 1,
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            node_id: Some("node-A".to_string()),
            stage_attempt_id: Some("attempt-1".to_string()),
            event_kind: "stage_started".to_string(),
            timestamp: "2026-02-10T10:00:00Z".to_string(),
            payload: serde_json::json!({"status":"ok"}),
            correlation: CorrelationMetadata {
                sequence_no: Some(1),
                ..CorrelationMetadata::default()
            },
        };

        let encoded = serde_json::to_vec(&envelope).expect("envelope should serialize");
        let decoded: StoredTurnEnvelope =
            serde_json::from_slice(&encoded).expect("envelope should deserialize");

        assert_eq!(decoded, envelope);
    }
}
