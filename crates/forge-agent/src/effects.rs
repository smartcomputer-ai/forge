//! Effect intent, receipt, and stream-frame records.
//!
//! Effects are the core model's representation of work that runners and
//! adapters perform outside deterministic state reduction.

use crate::context::{CompactionStrategy, LlmTokenCountRecord, LlmUsageRecord};
use crate::ids::{CorrelationId, EffectId, RunId, SessionId, SubmissionId, ToolCallId, TurnId};
use crate::refs::ArtifactRef;
use crate::tooling::ToolCallObserved;
use crate::turn::ResolvedTurnContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectMetadata {
    pub submission_id: Option<SubmissionId>,
    pub correlation_id: Option<CorrelationId>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEffectIntent {
    pub effect_id: EffectId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub kind: AgentEffectKind,
    pub emitted_at_ms: u64,
    pub metadata: EffectMetadata,
}

impl AgentEffectIntent {
    pub fn new(
        effect_id: EffectId,
        session_id: SessionId,
        kind: AgentEffectKind,
        emitted_at_ms: u64,
    ) -> Self {
        Self {
            effect_id,
            session_id,
            run_id: None,
            turn_id: None,
            kind,
            emitted_at_ms,
            metadata: EffectMetadata::default(),
        }
    }

    pub fn receipt(&self, kind: AgentReceiptKind, completed_at_ms: u64) -> AgentEffectReceipt {
        AgentEffectReceipt {
            effect_id: self.effect_id.clone(),
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            turn_id: self.turn_id.clone(),
            kind,
            completed_at_ms,
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AgentEffectKind {
    LlmComplete(LlmGenerationRequest),
    LlmStream(LlmGenerationRequest),
    LlmCountTokens(LlmCountTokensRequest),
    LlmCompact(LlmCompactRequest),
    McpCall(McpCallRequest),
    Confirmation(ConfirmationRequest),
    ToolInvoke(ToolInvocationRequest),
    Subagent(SubagentRequest),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmGenerationRequest {
    pub resolved_context: ResolvedTurnContext,
    pub request_ref: Option<ArtifactRef>,
    pub stream: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCountTokensRequest {
    pub resolved_context: ResolvedTurnContext,
    pub candidate_plan_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCompactRequest {
    pub resolved_context: ResolvedTurnContext,
    pub source_items: Vec<ArtifactRef>,
    pub strategy: CompactionStrategy,
    pub source_range_start: Option<u64>,
    pub source_range_end: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCallRequest {
    pub server_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub arguments_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationRequest {
    pub call_id: ToolCallId,
    pub provider_call_id: Option<String>,
    pub tool_id: Option<String>,
    pub tool_name: String,
    pub arguments_json: Option<String>,
    pub arguments_ref: Option<ArtifactRef>,
    pub handler_id: Option<String>,
    pub context_ref: Option<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationRequest {
    pub request_id: String,
    pub prompt_ref: ArtifactRef,
    pub response_schema_ref: Option<ArtifactRef>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum SubagentRequest {
    Spawn {
        parent_session_id: SessionId,
        task_ref: ArtifactRef,
        role: Option<String>,
        inherited_context_refs: Vec<ArtifactRef>,
    },
    Send {
        child_session_id: SessionId,
        input_ref: ArtifactRef,
    },
    Wait {
        child_session_id: SessionId,
        timeout_ms: Option<u64>,
    },
    Interrupt {
        child_session_id: SessionId,
        reason_ref: Option<ArtifactRef>,
    },
    Close {
        child_session_id: SessionId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEffectReceipt {
    pub effect_id: EffectId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub kind: AgentReceiptKind,
    pub completed_at_ms: u64,
    pub metadata: EffectMetadata,
}

impl AgentEffectReceipt {
    pub fn is_failure(&self) -> bool {
        matches!(self.kind, AgentReceiptKind::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AgentReceiptKind {
    LlmComplete(LlmGenerationReceipt),
    LlmStream(LlmGenerationReceipt),
    LlmCountTokens(LlmCountTokensReceipt),
    LlmCompact(LlmCompactReceipt),
    McpCall(McpCallReceipt),
    Confirmation(ConfirmationReceipt),
    ToolInvoke(ToolInvocationReceipt),
    Subagent(SubagentReceipt),
    Failed(EffectFailure),
    Cancelled(EffectCancellation),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryMetadata {
    pub attempt: u64,
    pub max_attempts: Option<u64>,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmGenerationReceipt {
    pub assistant_message_ref: Option<ArtifactRef>,
    pub reasoning_summary_ref: Option<ArtifactRef>,
    pub raw_provider_response_ref: Option<ArtifactRef>,
    pub tool_calls: Vec<ToolCallObserved>,
    pub usage: Option<LlmUsageRecord>,
    pub finish_reason: Option<String>,
    pub provider_response_id: Option<String>,
    pub retry: Option<RetryMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCountTokensReceipt {
    pub token_count: LlmTokenCountRecord,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCompactReceipt {
    pub artifact_refs: Vec<ArtifactRef>,
    pub warnings: Vec<String>,
    pub usage: Option<LlmUsageRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCallReceipt {
    pub result_ref: Option<ArtifactRef>,
    pub is_error: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationReceipt {
    pub call_id: ToolCallId,
    pub tool_id: Option<String>,
    pub tool_name: String,
    pub output_ref: Option<ArtifactRef>,
    pub model_visible_output_ref: Option<ArtifactRef>,
    pub is_error: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmationReceipt {
    pub request_id: String,
    pub response_ref: ArtifactRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum SubagentReceipt {
    Running {
        child_session_id: SessionId,
    },
    Completed {
        child_session_id: SessionId,
        output_ref: Option<ArtifactRef>,
    },
    Interrupted {
        child_session_id: SessionId,
        reason_ref: Option<ArtifactRef>,
    },
    Errored {
        child_session_id: Option<SessionId>,
        failure: EffectFailure,
    },
    Shutdown {
        child_session_id: SessionId,
    },
    NotFound {
        child_session_id: SessionId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectFailure {
    pub code: String,
    pub detail: String,
    pub retryable: bool,
    pub failure_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectCancellation {
    pub reason: Option<String>,
    pub reason_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectStreamFrame {
    pub effect_id: EffectId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub kind: EffectStreamFrameKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EffectStreamFrameKind {
    TextDelta {
        delta: String,
    },
    ReasoningDelta {
        delta: String,
    },
    ToolOutputDelta {
        call_id: ToolCallId,
        delta: String,
    },
    Progress {
        message: String,
        progress: Option<u64>,
        total: Option<u64>,
    },
    Artifact {
        artifact_ref: ArtifactRef,
    },
    Raw {
        frame_ref: ArtifactRef,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::IdAllocator;

    #[test]
    fn effect_intent_carries_idempotency_key_into_receipt() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let effect_id = ids.allocate_effect_id();
        let intent = AgentEffectIntent::new(
            effect_id.clone(),
            ids.session_id.clone(),
            AgentEffectKind::ToolInvoke(ToolInvocationRequest {
                call_id: ToolCallId::new("call-1"),
                provider_call_id: None,
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                arguments_json: Some(r#"{"text":"hi"}"#.into()),
                arguments_ref: None,
                handler_id: Some("test.echo".into()),
                context_ref: None,
                metadata: BTreeMap::new(),
            }),
            10,
        );

        let receipt = intent.receipt(
            AgentReceiptKind::ToolInvoke(ToolInvocationReceipt {
                call_id: ToolCallId::new("call-1"),
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                output_ref: None,
                model_visible_output_ref: None,
                is_error: false,
                metadata: BTreeMap::new(),
            }),
            11,
        );

        assert_eq!(receipt.effect_id, effect_id);
        assert_eq!(receipt.session_id, ids.session_id);
        assert!(!receipt.is_failure());
    }

    #[test]
    fn failed_receipt_is_a_settled_payload() {
        let receipt = AgentEffectReceipt {
            effect_id: EffectId {
                session_id: SessionId::new("session-a"),
                effect_seq: 1,
            },
            session_id: SessionId::new("session-a"),
            run_id: None,
            turn_id: None,
            kind: AgentReceiptKind::Failed(EffectFailure {
                code: "adapter_error".into(),
                detail: "process failed".into(),
                retryable: false,
                failure_ref: None,
            }),
            completed_at_ms: 12,
            metadata: EffectMetadata::default(),
        };

        assert!(receipt.is_failure());
    }

    #[test]
    fn generic_tool_invocation_effect_round_trips() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let intent = AgentEffectIntent::new(
            ids.allocate_effect_id(),
            ids.session_id.clone(),
            AgentEffectKind::ToolInvoke(ToolInvocationRequest {
                call_id: ToolCallId::new("call-1"),
                provider_call_id: Some("provider-call-1".into()),
                tool_id: Some("tool.fs.read".into()),
                tool_name: "read_file".into(),
                arguments_json: Some(r#"{"path":"README.md"}"#.into()),
                arguments_ref: None,
                handler_id: Some("local.fs".into()),
                context_ref: None,
                metadata: BTreeMap::new(),
            }),
            10,
        );

        let encoded = serde_json::to_string(&intent).expect("serialize intent");
        let decoded: AgentEffectIntent = serde_json::from_str(&encoded).expect("decode intent");

        assert_eq!(decoded, intent);
    }

    #[test]
    fn stream_frame_round_trips_through_json() {
        let frame = EffectStreamFrame {
            effect_id: EffectId {
                session_id: SessionId::new("session-a"),
                effect_seq: 1,
            },
            session_id: SessionId::new("session-a"),
            run_id: None,
            turn_id: None,
            sequence: 1,
            observed_at_ms: 10,
            kind: EffectStreamFrameKind::TextDelta { delta: "hi".into() },
        };

        let encoded = serde_json::to_string(&frame).expect("serialize frame");
        let decoded: EffectStreamFrame = serde_json::from_str(&encoded).expect("decode frame");
        assert_eq!(decoded, frame);
    }
}
