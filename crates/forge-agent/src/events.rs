//! Agent event records.
//!
//! This module will contain input, lifecycle, effect, and observation event
//! families. Human-facing CLI/web output is a projection over these events.

use crate::batch::ToolCallStatus;
use crate::config::{RunConfigOverride, SessionConfig, TurnConfig};
use crate::context::{ContextOperationState, ContextPressureRecord, LlmUsageRecord};
use crate::effects::{AgentEffectIntent, AgentEffectReceipt, EffectStreamFrame};
use crate::ids::{
    CorrelationId, EffectId, JournalSeq, ProjectionItemId, RunId, SessionId, SubmissionId,
    ToolBatchId, ToolCallId, TurnId,
};
use crate::lifecycle::{RunLifecycle, SessionStatus, TurnLifecycle};
use crate::refs::{ArtifactRef, TranscriptRef};
use crate::tooling::{ToolCallObserved, ToolProfile, ToolRegistry};
use crate::transcript::TranscriptRange;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const AGENT_EVENT_RECORD_KIND: &str = "forge.agent.runtime.v2.journal_event";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEventJoins {
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub effect_id: Option<EffectId>,
    pub tool_batch_id: Option<ToolBatchId>,
    pub tool_call_id: Option<ToolCallId>,
    pub submission_id: Option<SubmissionId>,
    pub correlation_id: Option<CorrelationId>,
    pub parent_event_id: Option<String>,
    pub parent_effect_id: Option<EffectId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub event_id: String,
    pub journal_seq: Option<JournalSeq>,
    pub session_id: SessionId,
    pub joins: AgentEventJoins,
    pub observed_at_ms: u64,
    pub kind: AgentEventKind,
}

impl AgentEvent {
    pub fn new(
        event_id: impl Into<String>,
        session_id: SessionId,
        observed_at_ms: u64,
        kind: AgentEventKind,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            journal_seq: None,
            session_id,
            joins: AgentEventJoins::default(),
            observed_at_ms,
            kind,
        }
    }

    pub fn with_journal_seq(mut self, journal_seq: JournalSeq) -> Self {
        self.journal_seq = Some(journal_seq);
        self
    }

    pub fn with_joins(mut self, joins: AgentEventJoins) -> Self {
        self.joins = joins;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "family")]
pub enum AgentEventKind {
    Input(InputEvent),
    Lifecycle(LifecycleEvent),
    Effect(EffectEvent),
    Observation(ObservationEvent),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum InputEvent {
    SessionOpened {
        config: Option<SessionConfig>,
    },
    SessionConfigUpdated {
        config: SessionConfig,
    },
    RunRequested {
        input_ref: ArtifactRef,
        run_overrides: Option<RunConfigOverride>,
    },
    FollowUpInputAppended {
        input_ref: ArtifactRef,
        run_overrides: Option<RunConfigOverride>,
    },
    RunSteerRequested {
        instruction_ref: ArtifactRef,
    },
    RunInterruptRequested {
        reason_ref: Option<ArtifactRef>,
    },
    SessionPaused,
    SessionResumed,
    SessionClosed,
    TurnContextOverrideRequested {
        turn_id: Option<TurnId>,
        override_: TurnConfig,
    },
    SessionHistoryRewriteRequested {
        request: HistoryRewriteRequest,
    },
    SessionHistoryRollbackRequested {
        request: HistoryRollbackRequest,
    },
    ToolRegistrySet {
        registry: ToolRegistry,
    },
    ToolProfileSelected {
        profile_id: String,
    },
    ToolOverridesSet {
        overrides: ToolOverrides,
    },
    ConfirmationProvided {
        request_id: String,
        response_ref: ArtifactRef,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRewriteRequest {
    pub rewrite_id: String,
    pub cause: String,
    pub source_range: Option<TranscriptRange>,
    pub replacement_transcript_ref: Option<TranscriptRef>,
    pub replacement_artifact_refs: Vec<ArtifactRef>,
    pub filesystem_changes_affected: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRollbackRequest {
    pub rollback_id: String,
    pub user_turns: u64,
    pub reason: Option<String>,
    pub reason_ref: Option<ArtifactRef>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOverrideScope {
    #[default]
    Session,
    Run,
    Turn,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOverrides {
    pub scope: ToolOverrideScope,
    pub profile: Option<ToolProfile>,
    pub enable: Vec<String>,
    pub disable: Vec<String>,
    pub force: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LifecycleEvent {
    SessionLifecycleChanged {
        from: SessionStatus,
        to: SessionStatus,
    },
    SessionStatusChanged {
        from: SessionStatus,
        to: SessionStatus,
    },
    RunLifecycleChanged {
        run_id: RunId,
        from: RunLifecycle,
        to: RunLifecycle,
    },
    TurnStarted {
        turn_id: TurnId,
    },
    TurnCompleted {
        turn_id: TurnId,
    },
    TurnFailed {
        turn_id: TurnId,
        failure_ref: Option<ArtifactRef>,
    },
    TurnLifecycleChanged {
        turn_id: TurnId,
        from: TurnLifecycle,
        to: TurnLifecycle,
    },
    ToolBatchStarted {
        tool_batch_id: ToolBatchId,
    },
    ToolBatchCompleted {
        tool_batch_id: ToolBatchId,
        results_ref: Option<ArtifactRef>,
    },
    ContextOperationStarted {
        operation: ContextOperationState,
    },
    ContextOperationCompleted {
        operation: ContextOperationState,
    },
    ContextPressureRecorded {
        pressure: ContextPressureRecord,
    },
    HistoryRewriteCompleted {
        rewrite_id: String,
        resulting_transcript_ref: Option<TranscriptRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EffectEvent {
    EffectIntentRecorded { intent: AgentEffectIntent },
    EffectReceiptRecorded { receipt: AgentEffectReceipt },
    EffectStreamFrameObserved { frame: EffectStreamFrame },
}

impl EffectEvent {
    pub fn effect_id(&self) -> &EffectId {
        match self {
            Self::EffectIntentRecorded { intent } => &intent.effect_id,
            Self::EffectReceiptRecorded { receipt } => &receipt.effect_id,
            Self::EffectStreamFrameObserved { frame } => &frame.effect_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ObservationEvent {
    UserMessageObserved {
        message_ref: ArtifactRef,
        preview: Option<String>,
    },
    AssistantMessageObserved {
        message_ref: Option<ArtifactRef>,
        preview: Option<String>,
    },
    ReasoningObserved {
        reasoning_ref: ArtifactRef,
        preview: Option<String>,
    },
    ToolCallObserved {
        call: ToolCallObserved,
    },
    ToolOutputObserved {
        call_id: ToolCallId,
        status: ToolCallStatus,
        output_ref: Option<ArtifactRef>,
        model_visible_output_ref: Option<ArtifactRef>,
    },
    FileChangeObserved {
        change: FileChangeObservation,
    },
    ProjectionItemObserved {
        item_id: ProjectionItemId,
        item_kind: String,
        item_ref: Option<ArtifactRef>,
    },
    TokenUsageObserved {
        usage: LlmUsageRecord,
    },
    WarningObserved {
        code: String,
        message: String,
        detail_ref: Option<ArtifactRef>,
    },
    CostObserved {
        amount_micros: u64,
        currency: String,
        metadata: BTreeMap<String, String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeObservation {
    pub path: String,
    pub change_kind: FileChangeKind,
    pub before_ref: Option<ArtifactRef>,
    pub after_ref: Option<ArtifactRef>,
    pub patch_ref: Option<ArtifactRef>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
    Renamed,
    #[default]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{AgentEffectKind, ToolInvocationRequest};
    use crate::ids::{EffectId, IdAllocator};
    use crate::refs::ArtifactRef;
    use std::collections::BTreeMap;

    #[test]
    fn effect_event_exposes_effect_id_for_all_phases() {
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
                arguments_json: None,
                arguments_ref: None,
                handler_id: Some("test.echo".into()),
                context_ref: None,
                metadata: BTreeMap::new(),
            }),
            10,
        );
        let event = EffectEvent::EffectIntentRecorded { intent };

        assert_eq!(event.effect_id(), &effect_id);
    }

    #[test]
    fn input_event_round_trips_without_hook_or_policy_variants() {
        let event = AgentEvent::new(
            "event-1",
            SessionId::new("session-a"),
            10,
            AgentEventKind::Input(InputEvent::RunRequested {
                input_ref: ArtifactRef::new("blob://prompt"),
                run_overrides: None,
            }),
        );

        let encoded = serde_json::to_string(&event).expect("serialize event");
        assert!(!encoded.contains("hook"));
        assert!(!encoded.contains("approval"));
        assert!(!encoded.contains("permission"));
        let decoded: AgentEvent = serde_json::from_str(&encoded).expect("decode event");
        assert_eq!(decoded, event);
    }

    #[test]
    fn journal_event_carries_sequence_and_causality_refs() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let effect_id = ids.allocate_effect_id();
        let event = AgentEvent::new(
            "event-1",
            ids.session_id.clone(),
            10,
            AgentEventKind::Effect(EffectEvent::EffectStreamFrameObserved {
                frame: EffectStreamFrame {
                    effect_id: effect_id.clone(),
                    session_id: ids.session_id.clone(),
                    run_id: None,
                    turn_id: None,
                    sequence: 1,
                    observed_at_ms: 10,
                    kind: crate::effects::EffectStreamFrameKind::Progress {
                        message: "working".into(),
                        progress: None,
                        total: None,
                    },
                },
            }),
        )
        .with_journal_seq(ids.allocate_journal_seq())
        .with_joins(AgentEventJoins {
            effect_id: Some(effect_id.clone()),
            parent_effect_id: Some(effect_id.clone()),
            ..Default::default()
        });

        assert_eq!(event.journal_seq, Some(JournalSeq(1)));
        assert_eq!(event.joins.effect_id, Some(effect_id.clone()));
        assert_eq!(event.joins.parent_effect_id, Some(effect_id.clone()));
        let AgentEventKind::Effect(effect_event) = &event.kind else {
            panic!("expected effect event");
        };
        assert_eq!(effect_event.effect_id(), &effect_id);
    }

    #[test]
    fn observation_event_can_reference_projection_item() {
        let event = ObservationEvent::ProjectionItemObserved {
            item_id: ProjectionItemId {
                session_id: SessionId::new("session-a"),
                item_seq: 1,
            },
            item_kind: "assistant".into(),
            item_ref: None,
        };

        let encoded = serde_json::to_string(&event).expect("serialize observation");
        assert!(encoded.contains("projection_item_observed"));
    }

    #[test]
    fn receipt_effect_event_uses_receipt_effect_id() {
        let receipt = AgentEffectReceipt {
            effect_id: EffectId {
                session_id: SessionId::new("session-a"),
                effect_seq: 7,
            },
            session_id: SessionId::new("session-a"),
            run_id: None,
            turn_id: None,
            kind: crate::effects::AgentReceiptKind::Cancelled(crate::effects::EffectCancellation {
                reason: Some("stop".into()),
                reason_ref: None,
            }),
            completed_at_ms: 12,
            metadata: crate::effects::EffectMetadata::default(),
        };
        let event = EffectEvent::EffectReceiptRecorded { receipt };

        assert_eq!(event.effect_id().effect_seq, 7);
    }
}
