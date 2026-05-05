//! Session and run state records.
//!
//! This module will contain deterministic core state, pending effects, run
//! queues, transcript lineage, fork metadata, and history rewrite state.

use crate::batch::ActiveToolBatch;
use crate::config::{RunConfig, RunConfigOverride, SessionConfig};
use crate::context::{ContextState, LlmUsageRecord};
use crate::effects::{AgentEffectIntent, AgentEffectReceipt};
use crate::error::ModelError;
use crate::ids::{EffectId, IdAllocator, RunId, SessionId, SubmissionId, TurnId};
use crate::lifecycle::{RunLifecycle, SessionStatus};
use crate::refs::{ArtifactRef, TranscriptBoundary, TranscriptRef};
use crate::subagent::SubagentRecord;
use crate::tooling::{ToolRegistry, ToolRuntimeContext};
use crate::transcript::TranscriptRange;
use crate::turn::TurnPlan;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CauseRef {
    pub kind: String,
    pub id: String,
    pub ref_: Option<ArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RunCauseOrigin {
    DirectSubmission {
        submission_id: Option<SubmissionId>,
        source: String,
        request_ref: Option<ArtifactRef>,
    },
    DomainEvent {
        schema: String,
        event_ref: Option<ArtifactRef>,
        key: Option<String>,
    },
    Internal {
        reason: String,
        ref_: Option<ArtifactRef>,
    },
}

impl Default for RunCauseOrigin {
    fn default() -> Self {
        Self::Internal {
            reason: String::new(),
            ref_: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCause {
    pub kind: String,
    pub origin: RunCauseOrigin,
    pub input_refs: Vec<ArtifactRef>,
    pub payload_schema: Option<String>,
    pub payload_ref: Option<ArtifactRef>,
    pub subject_refs: Vec<CauseRef>,
}

impl RunCause {
    pub fn direct_input(input_ref: ArtifactRef, submission_id: Option<SubmissionId>) -> Self {
        Self {
            kind: "forge.agent/user_input".into(),
            origin: RunCauseOrigin::DirectSubmission {
                submission_id,
                source: "RunRequested".into(),
                request_ref: None,
            },
            input_refs: vec![input_ref],
            payload_schema: None,
            payload_ref: None,
            subject_refs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFailure {
    pub code: String,
    pub detail: String,
    pub failure_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub output_ref: Option<ArtifactRef>,
    pub failure: Option<RunFailure>,
    pub cancelled_reason: Option<String>,
    pub interrupted_reason_ref: Option<ArtifactRef>,
}

impl RunOutcome {
    pub fn completed(output_ref: Option<ArtifactRef>) -> Self {
        Self {
            output_ref,
            ..Default::default()
        }
    }

    pub fn failed(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            failure: Some(RunFailure {
                code: code.into(),
                detail: detail.into(),
                failure_ref: None,
            }),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEffectRecord {
    pub intent: AgentEffectIntent,
    pub status: PendingEffectStatus,
    pub receipt: Option<AgentEffectReceipt>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingEffectStatus {
    #[default]
    Pending,
    Streaming,
    Settled,
    Abandoned,
}

impl PendingEffectStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Abandoned)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedRunInput {
    pub submission_id: Option<SubmissionId>,
    pub input_ref: ArtifactRef,
    pub run_overrides: Option<RunConfigOverride>,
    pub queued_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedSteeringInput {
    pub instruction_ref: ArtifactRef,
    pub queued_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingHumanRequest {
    pub request_id: String,
    pub prompt_ref: ArtifactRef,
    pub response_schema_ref: Option<ArtifactRef>,
    pub requested_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMetadata {
    pub name: Option<String>,
    pub memory_mode: Option<String>,
    pub external_links: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: RunId,
    pub lifecycle: RunLifecycle,
    pub cause: RunCause,
    pub config: RunConfig,
    pub input_refs: Vec<ArtifactRef>,
    pub current_turn_plan: Option<TurnPlan>,
    pub active_turn_id: Option<TurnId>,
    pub active_llm_effect_id: Option<EffectId>,
    pub completed_tool_batches: Vec<ActiveToolBatch>,
    pub active_tool_batch: Option<ActiveToolBatch>,
    pub pending_effects: BTreeMap<EffectId, PendingEffectRecord>,
    pub latest_output_ref: Option<ArtifactRef>,
    pub usage_records: Vec<LlmUsageRecord>,
    pub run_trace_ref: Option<ArtifactRef>,
    pub outcome: Option<RunOutcome>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}

impl RunState {
    pub fn queued(run_id: RunId, cause: RunCause, config: RunConfig, queued_at_ms: u64) -> Self {
        Self {
            input_refs: cause.input_refs.clone(),
            run_id,
            lifecycle: RunLifecycle::Queued,
            cause,
            config,
            started_at_ms: queued_at_ms,
            updated_at_ms: queued_at_ms,
            ..Default::default()
        }
    }

    pub fn transition_to(&mut self, next: RunLifecycle, at_ms: u64) -> Result<(), ModelError> {
        self.lifecycle = self.lifecycle.transition_to(next)?;
        self.updated_at_ms = at_ms;
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        self.lifecycle.is_terminal()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: RunId,
    pub lifecycle: RunLifecycle,
    pub cause: RunCause,
    pub input_refs: Vec<ArtifactRef>,
    pub completed_tool_batches: Vec<ActiveToolBatch>,
    pub outcome: Option<RunOutcome>,
    pub usage_records: Vec<LlmUsageRecord>,
    pub run_trace_ref: Option<ArtifactRef>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

impl From<RunState> for RunRecord {
    fn from(run: RunState) -> Self {
        Self {
            run_id: run.run_id,
            lifecycle: run.lifecycle,
            cause: run.cause,
            input_refs: run.input_refs,
            completed_tool_batches: run.completed_tool_batches,
            outcome: run.outcome,
            usage_records: run.usage_records,
            run_trace_ref: run.run_trace_ref,
            started_at_ms: run.started_at_ms,
            ended_at_ms: run.updated_at_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SessionSource {
    Empty,
    TranscriptPrefix {
        transcript_ref: TranscriptRef,
        boundary: Option<TranscriptBoundary>,
    },
    TranscriptSnapshot {
        transcript_ref: TranscriptRef,
    },
    ParentSessionRun {
        parent_session_id: SessionId,
        parent_run_id: Option<RunId>,
        inherited_context_refs: Vec<ArtifactRef>,
    },
    ImportedHistory {
        transcript_ref: TranscriptRef,
    },
}

impl Default for SessionSource {
    fn default() -> Self {
        Self::Empty
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLineage {
    pub source: SessionSource,
    pub fork_reason: Option<String>,
    pub created_from_event_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRewriteRecord {
    pub rewrite_id: String,
    pub cause: String,
    pub source_range: Option<TranscriptRange>,
    pub replacement_transcript_ref: Option<TranscriptRef>,
    pub replacement_artifact_refs: Vec<ArtifactRef>,
    pub filesystem_changes_affected: Option<bool>,
    pub resulting_active_boundary: Option<TranscriptBoundary>,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRollbackRecord {
    pub rollback_id: String,
    pub source_range: Option<TranscriptRange>,
    pub user_turns: u64,
    pub reason: Option<String>,
    pub reason_ref: Option<ArtifactRef>,
    pub resulting_active_boundary: Option<TranscriptBoundary>,
    pub recorded_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: SessionId,
    pub status: SessionStatus,
    pub config: SessionConfig,
    pub id_allocator: IdAllocator,
    pub context_state: ContextState,
    pub tool_registry: ToolRegistry,
    pub selected_tool_profile: Option<String>,
    pub tool_runtime_context: ToolRuntimeContext,
    pub current_run: Option<RunState>,
    pub run_history: Vec<RunRecord>,
    pub pending_follow_up_inputs: VecDeque<QueuedRunInput>,
    pub pending_steering_inputs: VecDeque<QueuedSteeringInput>,
    pub pending_human_requests: BTreeMap<String, PendingHumanRequest>,
    pub pending_effects: BTreeMap<EffectId, PendingEffectRecord>,
    pub transcript_refs: Vec<TranscriptRef>,
    pub artifact_refs: Vec<ArtifactRef>,
    pub lineage: Option<SessionLineage>,
    pub history_rewrites: Vec<HistoryRewriteRecord>,
    pub history_rollbacks: Vec<HistoryRollbackRecord>,
    pub subagents: BTreeMap<SessionId, SubagentRecord>,
    pub thread_metadata: ThreadMetadata,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl SessionState {
    pub fn new(session_id: SessionId, config: SessionConfig, created_at_ms: u64) -> Self {
        Self {
            id_allocator: IdAllocator::new(session_id.clone()),
            session_id,
            status: SessionStatus::New,
            config,
            context_state: ContextState::default(),
            tool_registry: ToolRegistry::default(),
            selected_tool_profile: None,
            tool_runtime_context: ToolRuntimeContext::default(),
            current_run: None,
            run_history: Vec::new(),
            pending_follow_up_inputs: VecDeque::new(),
            pending_steering_inputs: VecDeque::new(),
            pending_human_requests: BTreeMap::new(),
            pending_effects: BTreeMap::new(),
            transcript_refs: Vec::new(),
            artifact_refs: Vec::new(),
            lineage: None,
            history_rewrites: Vec::new(),
            history_rollbacks: Vec::new(),
            subagents: BTreeMap::new(),
            thread_metadata: ThreadMetadata::default(),
            created_at_ms,
            updated_at_ms: created_at_ms,
        }
    }

    pub fn with_lineage(mut self, lineage: SessionLineage) -> Self {
        self.lineage = Some(lineage);
        self
    }

    pub fn transition_status(&mut self, next: SessionStatus, at_ms: u64) -> Result<(), ModelError> {
        self.status = self.status.transition_to(next)?;
        self.updated_at_ms = at_ms;
        Ok(())
    }

    pub fn can_start_foreground_run(&self) -> bool {
        self.status.accepts_new_runs() && self.current_run.is_none()
    }

    pub fn start_run(&mut self, mut run: RunState, at_ms: u64) -> Result<(), ModelError> {
        if self.current_run.is_some() {
            return Err(ModelError::InvalidValue {
                field: "current_run",
                message: "foreground run already active".into(),
            });
        }
        if !self.status.accepts_new_runs() {
            return Err(ModelError::InvalidValue {
                field: "status",
                message: "session does not accept new runs".into(),
            });
        }
        run.transition_to(RunLifecycle::Running, at_ms)?;
        self.current_run = Some(run);
        self.updated_at_ms = at_ms;
        Ok(())
    }

    pub fn finish_current_run(
        &mut self,
        lifecycle: RunLifecycle,
        outcome: RunOutcome,
        at_ms: u64,
    ) -> Result<RunRecord, ModelError> {
        if !lifecycle.is_terminal() {
            return Err(ModelError::InvalidValue {
                field: "lifecycle",
                message: "finished run lifecycle must be terminal".into(),
            });
        }
        let Some(mut run) = self.current_run.take() else {
            return Err(ModelError::InvalidValue {
                field: "current_run",
                message: "no foreground run is active".into(),
            });
        };
        run.transition_to(lifecycle, at_ms)?;
        run.outcome = Some(outcome);
        run.updated_at_ms = at_ms;
        let record = RunRecord::from(run);
        self.run_history.push(record.clone());
        self.updated_at_ms = at_ms;
        Ok(record)
    }

    pub fn enqueue_follow_up(&mut self, input: QueuedRunInput) {
        self.pending_follow_up_inputs.push_back(input);
    }

    pub fn enqueue_steering(&mut self, input: QueuedSteeringInput) {
        self.pending_steering_inputs.push_back(input);
    }

    pub fn record_pending_effect(&mut self, intent: AgentEffectIntent) {
        let record = PendingEffectRecord {
            intent: intent.clone(),
            status: PendingEffectStatus::Pending,
            receipt: None,
        };
        self.pending_effects
            .insert(intent.effect_id.clone(), record);
    }

    pub fn settle_pending_effect(&mut self, receipt: AgentEffectReceipt) {
        if let Some(record) = self.pending_effects.get_mut(&receipt.effect_id) {
            record.status = PendingEffectStatus::Settled;
            record.receipt = Some(receipt);
        }
    }

    pub fn record_history_rewrite(&mut self, record: HistoryRewriteRecord) {
        self.history_rewrites.push(record);
    }

    pub fn record_history_rollback(&mut self, record: HistoryRollbackRecord) {
        self.history_rollbacks.push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{AgentEffectKind, AgentReceiptKind, ArtifactGetRequest, ArtifactReceipt};
    use crate::refs::{ArtifactKind, ArtifactRef, TranscriptRef, TranscriptRefKind};

    fn active_session() -> SessionState {
        let mut session =
            SessionState::new(SessionId::new("session-a"), SessionConfig::default(), 1);
        session
            .transition_status(SessionStatus::Active, 2)
            .expect("activate session");
        session
    }

    #[test]
    fn session_state_represents_new_and_active_run() {
        let mut session = active_session();
        let run_id = session.id_allocator.allocate_run_id();
        let input_ref = ArtifactRef::new("blob://prompt", ArtifactKind::UserPrompt);
        let cause = RunCause::direct_input(input_ref.clone(), Some(SubmissionId::new("submit-1")));
        let run = RunState::queued(
            run_id.clone(),
            cause,
            RunConfig::from_session(&session.config, None),
            3,
        );

        session.start_run(run, 4).expect("start run");

        let current = session.current_run.as_ref().expect("current run");
        assert_eq!(current.run_id, run_id);
        assert_eq!(current.lifecycle, RunLifecycle::Running);
        assert_eq!(current.input_refs, vec![input_ref]);
    }

    #[test]
    fn session_state_moves_completed_run_to_history() {
        let mut session = active_session();
        let run_id = session.id_allocator.allocate_run_id();
        let run = RunState::queued(
            run_id.clone(),
            RunCause::default(),
            RunConfig::from_session(&session.config, None),
            3,
        );
        session.start_run(run, 4).expect("start run");

        let record = session
            .finish_current_run(
                RunLifecycle::Completed,
                RunOutcome::completed(Some(ArtifactRef::new(
                    "blob://answer",
                    ArtifactKind::AssistantMessage,
                ))),
                5,
            )
            .expect("finish run");

        assert!(session.current_run.is_none());
        assert_eq!(session.run_history.len(), 1);
        assert_eq!(record.run_id, run_id);
        assert_eq!(record.lifecycle, RunLifecycle::Completed);
    }

    #[test]
    fn session_state_can_represent_interrupted_run() {
        let mut session = active_session();
        let run = RunState::queued(
            session.id_allocator.allocate_run_id(),
            RunCause::default(),
            RunConfig::from_session(&session.config, None),
            3,
        );
        session.start_run(run, 4).expect("start run");

        let record = session
            .finish_current_run(
                RunLifecycle::Interrupted,
                RunOutcome {
                    interrupted_reason_ref: Some(ArtifactRef::new(
                        "blob://reason",
                        ArtifactKind::Custom,
                    )),
                    ..Default::default()
                },
                5,
            )
            .expect("interrupt run");

        assert_eq!(record.lifecycle, RunLifecycle::Interrupted);
        assert!(
            record
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.interrupted_reason_ref.as_ref())
                .is_some()
        );
    }

    #[test]
    fn session_state_can_represent_fork_and_history_rewrite() {
        let lineage = SessionLineage {
            source: SessionSource::TranscriptPrefix {
                transcript_ref: TranscriptRef::new(
                    "transcript://source/prefix",
                    TranscriptRefKind::Prefix,
                ),
                boundary: Some(TranscriptBoundary {
                    entry_seq: Some(3),
                    event_id: None,
                }),
            },
            fork_reason: Some("alternate branch".into()),
            created_from_event_id: Some("event-1".into()),
        };
        let mut session = SessionState::new(SessionId::new("fork"), SessionConfig::default(), 1)
            .with_lineage(lineage);

        session.record_history_rewrite(HistoryRewriteRecord {
            rewrite_id: "rewrite-1".into(),
            cause: "compaction".into(),
            source_range: Some(TranscriptRange {
                start_seq: 0,
                end_seq: 3,
            }),
            replacement_transcript_ref: Some(TranscriptRef::new(
                "transcript://fork/compacted",
                TranscriptRefKind::CompactedSnapshot,
            )),
            resulting_active_boundary: Some(TranscriptBoundary {
                entry_seq: Some(1),
                event_id: None,
            }),
            recorded_at_ms: 10,
            ..Default::default()
        });

        assert!(matches!(
            session.lineage.as_ref().map(|lineage| &lineage.source),
            Some(SessionSource::TranscriptPrefix { .. })
        ));
        assert_eq!(session.history_rewrites.len(), 1);
        assert_eq!(
            session.history_rewrites[0].filesystem_changes_affected,
            None
        );
    }

    #[test]
    fn pending_effect_can_be_recorded_and_settled() {
        let mut session = active_session();
        let effect_id = session.id_allocator.allocate_effect_id();
        let intent = AgentEffectIntent::new(
            effect_id.clone(),
            session.session_id.clone(),
            AgentEffectKind::ArtifactGet(ArtifactGetRequest {
                artifact: ArtifactRef::new("blob://input", ArtifactKind::Custom),
            }),
            10,
        );
        session.record_pending_effect(intent.clone());
        let receipt = intent.receipt(
            AgentReceiptKind::ArtifactGet(ArtifactReceipt {
                artifact: ArtifactRef::new("blob://input", ArtifactKind::Custom),
            }),
            11,
        );
        session.settle_pending_effect(receipt);

        let record = session.pending_effects.get(&effect_id).expect("effect");
        assert_eq!(record.status, PendingEffectStatus::Settled);
        assert!(record.receipt.is_some());
    }
}
