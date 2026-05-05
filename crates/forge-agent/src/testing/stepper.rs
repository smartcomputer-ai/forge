//! Local deterministic stepper for tests and development harnesses.

use crate::context::{LlmTokenCountQuality, LlmTokenCountRecord};
use crate::decider::{DecideRequest, decide_next_with};
use crate::effects::{
    AgentEffectIntent, AgentEffectKind, AgentEffectReceipt, AgentReceiptKind, LlmCompactReceipt,
    LlmGenerationReceipt, ToolInvocationReceipt,
};
use crate::error::ModelError;
use crate::events::{AgentEvent, AgentEventJoins, AgentEventKind, EffectEvent};
use crate::journal::InMemoryJournal;
use crate::loop_projection::{ProjectionBuilder, ProjectionOutput};
use crate::planner::{DefaultTurnPlanner, TurnPlanner};
use crate::reducer::apply_event;
use crate::refs::BlobRef;
use crate::state::SessionState;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepperQuiescence {
    #[default]
    WaitingForInput,
    WaitingForConfirmation,
    WaitingForEffects,
    WaitingForContextPrerequisite,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepperDriveResult {
    pub appended_event_count: u64,
    pub executed_effect_count: u64,
    pub projection_item_count: u64,
    pub transcript_item_count: u64,
    pub quiescence: StepperQuiescence,
}

pub trait LocalEffectExecutor {
    fn execute(&mut self, intent: &AgentEffectIntent) -> Option<AgentEffectReceipt>;
}

#[derive(Clone, Debug)]
pub struct LocalStepper<E, P = DefaultTurnPlanner> {
    pub state: SessionState,
    pub journal: InMemoryJournal,
    pub projections: ProjectionBuilder,
    pub executor: E,
    pub planner: P,
    pub now_ms: u64,
}

impl<E: LocalEffectExecutor> LocalStepper<E, DefaultTurnPlanner> {
    pub fn new(state: SessionState, executor: E) -> Self {
        let journal = InMemoryJournal::new(state.session_id.clone());
        let projections = ProjectionBuilder::new(state.session_id.clone());
        Self {
            state,
            journal,
            projections,
            executor,
            planner: DefaultTurnPlanner,
            now_ms: 1,
        }
    }
}

impl<E: LocalEffectExecutor, P: TurnPlanner> LocalStepper<E, P> {
    pub fn append_event(&mut self, event: AgentEvent) -> Result<ProjectionOutput, ModelError> {
        let appended = self.journal.append(event)?;
        apply_event(&mut self.state, &appended.event)?;
        Ok(self.projections.apply_event(&appended.event))
    }

    pub fn drive_until_quiescent(
        &mut self,
        max_steps: u64,
    ) -> Result<StepperDriveResult, ModelError> {
        let mut result = StepperDriveResult::default();
        for _ in 0..max_steps {
            let observed_at_ms = self.tick();
            let decision = decide_next_with(DecideRequest {
                state: &self.state,
                planner: &self.planner,
                observed_at_ms,
                stream_llm: false,
            })?;

            if decision.events.is_empty() && decision.intents.is_empty() {
                result.quiescence = classify_quiescence(&self.state);
                return Ok(result);
            }

            for event in decision.events {
                let projection = self.append_event(event)?;
                result.appended_event_count = result.appended_event_count.saturating_add(1);
                result.projection_item_count = result
                    .projection_item_count
                    .saturating_add(projection.projection_items.len() as u64);
                result.transcript_item_count = result
                    .transcript_item_count
                    .saturating_add(projection.transcript_items.len() as u64);
            }

            for intent in decision.intents {
                if let Some(receipt) = self.executor.execute(&intent) {
                    let projection = self.append_event(receipt_event(receipt))?;
                    result.appended_event_count = result.appended_event_count.saturating_add(1);
                    result.executed_effect_count = result.executed_effect_count.saturating_add(1);
                    result.projection_item_count = result
                        .projection_item_count
                        .saturating_add(projection.projection_items.len() as u64);
                    result.transcript_item_count = result
                        .transcript_item_count
                        .saturating_add(projection.transcript_items.len() as u64);
                }
            }
        }

        result.quiescence = classify_quiescence(&self.state);
        Ok(result)
    }

    fn tick(&mut self) -> u64 {
        self.now_ms = self.now_ms.saturating_add(1);
        self.now_ms
    }
}

pub fn classify_quiescence(state: &SessionState) -> StepperQuiescence {
    if !state.pending_confirmation_requests.is_empty() {
        return StepperQuiescence::WaitingForConfirmation;
    }
    if !state.pending_effects.is_empty()
        || state
            .current_run
            .as_ref()
            .is_some_and(|run| !run.pending_effects.is_empty())
    {
        return StepperQuiescence::WaitingForEffects;
    }
    if state
        .context_state
        .pending_context_operation
        .as_ref()
        .is_some_and(|operation| operation.blocks_generation())
    {
        return StepperQuiescence::WaitingForContextPrerequisite;
    }
    match state.run_history.last().map(|run| run.lifecycle) {
        Some(crate::lifecycle::RunLifecycle::Completed) if state.current_run.is_none() => {
            StepperQuiescence::Completed
        }
        Some(crate::lifecycle::RunLifecycle::Failed) if state.current_run.is_none() => {
            StepperQuiescence::Failed
        }
        Some(crate::lifecycle::RunLifecycle::Cancelled) if state.current_run.is_none() => {
            StepperQuiescence::Cancelled
        }
        Some(crate::lifecycle::RunLifecycle::Interrupted) if state.current_run.is_none() => {
            StepperQuiescence::Interrupted
        }
        _ => StepperQuiescence::WaitingForInput,
    }
}

fn receipt_event(receipt: AgentEffectReceipt) -> AgentEvent {
    AgentEvent::new(
        format!("receipt:{}", receipt.effect_id),
        receipt.session_id.clone(),
        receipt.completed_at_ms,
        AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded {
            receipt: receipt.clone(),
        }),
    )
    .with_joins(AgentEventJoins {
        run_id: receipt.run_id.clone(),
        turn_id: receipt.turn_id.clone(),
        effect_id: Some(receipt.effect_id.clone()),
        ..Default::default()
    })
}

#[derive(Clone, Debug, Default)]
pub struct FakeEffectExecutor {
    pub llm_receipts: VecDeque<LlmGenerationReceipt>,
    pub tool_receipts: BTreeMap<String, ToolInvocationReceipt>,
    pub executed_intents: Vec<AgentEffectIntent>,
}

impl FakeEffectExecutor {
    pub fn with_llm_receipts(receipts: impl IntoIterator<Item = LlmGenerationReceipt>) -> Self {
        Self {
            llm_receipts: receipts.into_iter().collect(),
            tool_receipts: BTreeMap::new(),
            executed_intents: Vec::new(),
        }
    }

    pub fn with_tool_receipt(
        mut self,
        call_id: impl Into<String>,
        receipt: ToolInvocationReceipt,
    ) -> Self {
        self.tool_receipts.insert(call_id.into(), receipt);
        self
    }
}

impl LocalEffectExecutor for FakeEffectExecutor {
    fn execute(&mut self, intent: &AgentEffectIntent) -> Option<AgentEffectReceipt> {
        self.executed_intents.push(intent.clone());
        match &intent.kind {
            AgentEffectKind::LlmComplete(_) | AgentEffectKind::LlmStream(_) => {
                let receipt = self.llm_receipts.pop_front()?;
                Some(intent.receipt(AgentReceiptKind::LlmComplete(receipt), intent.emitted_at_ms))
            }
            AgentEffectKind::ToolInvoke(request) => {
                let receipt = self
                    .tool_receipts
                    .remove(request.call_id.as_str())
                    .unwrap_or_else(|| ToolInvocationReceipt {
                        call_id: request.call_id.clone(),
                        tool_id: request.tool_id.clone(),
                        tool_name: request.tool_name.clone(),
                        output_ref: Some(BlobRef::from_bytes(
                            format!("tool-output:{}", request.call_id).as_bytes(),
                        )),
                        model_visible_output_ref: Some(BlobRef::from_bytes(
                            format!("tool-visible:{}", request.call_id).as_bytes(),
                        )),
                        is_error: false,
                        metadata: BTreeMap::new(),
                    });
                Some(intent.receipt(AgentReceiptKind::ToolInvoke(receipt), intent.emitted_at_ms))
            }
            AgentEffectKind::LlmCountTokens(request) => Some(intent.receipt(
                AgentReceiptKind::LlmCountTokens(crate::effects::LlmCountTokensReceipt {
                    token_count: LlmTokenCountRecord {
                        input_tokens: Some(
                            request.resolved_context.active_window_items.len() as u64 * 10,
                        ),
                        quality: LlmTokenCountQuality::LocalEstimate,
                        provider: request.resolved_context.provider.clone(),
                        model: request.resolved_context.model.clone(),
                        candidate_plan_id: request.candidate_plan_id.clone(),
                        counted_at_ms: intent.emitted_at_ms,
                        ..Default::default()
                    },
                }),
                intent.emitted_at_ms,
            )),
            AgentEffectKind::LlmCompact(_request) => Some(intent.receipt(
                AgentReceiptKind::LlmCompact(LlmCompactReceipt {
                    blob_refs: vec![BlobRef::from_bytes(
                        format!("compaction:{}", intent.effect_id).as_bytes(),
                    )],
                    warnings: Vec::new(),
                    usage: None,
                }),
                intent.emitted_at_ms,
            )),
            AgentEffectKind::McpCall(_)
            | AgentEffectKind::Confirmation(_)
            | AgentEffectKind::Subagent(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{
        CompactionStrategy, ContextOperationPhase, ContextOperationState, ContextPressureReason,
    };
    use crate::effects::LlmGenerationReceipt;
    use crate::events::{AgentEventKind, InputEvent, LifecycleEvent};
    use crate::ids::{SessionId, ToolCallId};
    use crate::refs::BlobRef;
    use crate::state::SessionState;
    use crate::tooling::{ToolCallObserved, ToolProfile, ToolRegistry, ToolSpec};
    use serde_json::json;

    fn stepper(executor: FakeEffectExecutor) -> LocalStepper<FakeEffectExecutor> {
        LocalStepper::new(
            SessionState::new(SessionId::new("session-a"), Default::default(), 1),
            executor,
        )
    }

    fn event(id: &str, at_ms: u64, kind: AgentEventKind) -> AgentEvent {
        AgentEvent::new(id, SessionId::new("session-a"), at_ms, kind)
    }

    fn open_and_request(stepper: &mut LocalStepper<FakeEffectExecutor>, input_ref: BlobRef) {
        stepper
            .append_event(event(
                "open",
                10,
                AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
            ))
            .expect("open");
        stepper
            .append_event(event(
                "run",
                11,
                AgentEventKind::Input(InputEvent::RunRequested {
                    input_ref,
                    run_overrides: None,
                }),
            ))
            .expect("run");
    }

    fn install_echo(stepper: &mut LocalStepper<FakeEffectExecutor>) {
        let mut registry = ToolRegistry::default();
        registry.insert_tool(ToolSpec::new(
            "echo",
            "echo",
            "Echo",
            json!({"type":"object"}),
        ));
        registry.insert_profile(ToolProfile {
            profile_id: "local".into(),
            tool_ids: vec!["echo".into()],
            ..Default::default()
        });
        stepper
            .append_event(event(
                "registry",
                12,
                AgentEventKind::Input(InputEvent::ToolRegistrySet { registry }),
            ))
            .expect("registry");
        stepper
            .append_event(event(
                "profile",
                13,
                AgentEventKind::Input(InputEvent::ToolProfileSelected {
                    profile_id: "local".into(),
                }),
            ))
            .expect("profile");
    }

    #[test]
    fn local_stepper_drives_fake_llm_run_to_completion_with_projections() {
        let mut stepper = stepper(FakeEffectExecutor::with_llm_receipts([
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://answer")),
                reasoning_summary_ref: Some(BlobRef::new_unchecked_for_tests("blob://reasoning")),
                ..Default::default()
            },
        ]));
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://prompt"),
        );
        stepper
            .append_event(event(
                "steer",
                12,
                AgentEventKind::Input(InputEvent::RunSteerRequested {
                    instruction_ref: BlobRef::new_unchecked_for_tests("blob://steer"),
                }),
            ))
            .expect("steer");

        let result = stepper.drive_until_quiescent(16).expect("drive");

        assert_eq!(result.quiescence, StepperQuiescence::Completed);
        assert!(stepper.state.current_run.is_none());
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(item.kind, crate::projection::ProjectionItemKind::User))
        );
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(item.kind, crate::projection::ProjectionItemKind::Assistant))
        );
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(item.kind, crate::projection::ProjectionItemKind::Reasoning))
        );
        assert!(
            stepper
                .executor
                .executed_intents
                .iter()
                .filter_map(|intent| match &intent.kind {
                    AgentEffectKind::LlmComplete(request) => Some(request),
                    _ => None,
                })
                .any(|request| request
                    .resolved_context
                    .active_window_items
                    .iter()
                    .any(|item| item.content_ref.as_str() == "blob://steer"))
        );
    }

    #[test]
    fn local_stepper_drives_fake_tool_round_trip_to_final_answer() {
        let mut stepper = stepper(FakeEffectExecutor::with_llm_receipts([
            LlmGenerationReceipt {
                tool_calls: vec![ToolCallObserved {
                    call_id: ToolCallId::new("call-1"),
                    tool_name: "echo".into(),
                    arguments_json: Some(r#"{"text":"hi"}"#.into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://final")),
                ..Default::default()
            },
        ]));
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://prompt"),
        );
        install_echo(&mut stepper);

        let result = stepper.drive_until_quiescent(32).expect("drive");

        assert_eq!(result.quiescence, StepperQuiescence::Completed);
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(
                    item.kind,
                    crate::projection::ProjectionItemKind::ToolCall { .. }
                ))
        );
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(
                    item.kind,
                    crate::projection::ProjectionItemKind::ToolOutput { .. }
                ))
        );
        assert!(
            stepper
                .projections
                .transcript_items
                .iter()
                .any(|item| item.kind == crate::transcript::TranscriptEntryKind::ToolResult)
        );
    }

    #[test]
    fn local_stepper_recovers_from_unavailable_tool_with_next_llm_turn() {
        let mut stepper = stepper(FakeEffectExecutor::with_llm_receipts([
            LlmGenerationReceipt {
                tool_calls: vec![ToolCallObserved {
                    call_id: ToolCallId::new("call-missing"),
                    tool_name: "missing".into(),
                    arguments_json: Some(r#"{"path":"nope"}"#.into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://recovery")),
                ..Default::default()
            },
        ]));
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://prompt"),
        );

        let result = stepper.drive_until_quiescent(32).expect("drive");

        assert_eq!(result.quiescence, StepperQuiescence::Completed);
        assert!(
            !stepper
                .executor
                .executed_intents
                .iter()
                .any(|intent| matches!(intent.kind, AgentEffectKind::ToolInvoke(_)))
        );
        assert!(
            stepper
                .state
                .run_history
                .first()
                .and_then(|run| run.outcome.as_ref())
                .and_then(|outcome| outcome.output_ref.as_ref())
                .is_some_and(|ref_| ref_.as_str() == "blob://recovery")
        );
    }

    #[test]
    fn local_stepper_promotes_follow_up_after_active_run_completes() {
        let mut stepper = stepper(FakeEffectExecutor::with_llm_receipts([
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://first")),
                ..Default::default()
            },
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://second")),
                ..Default::default()
            },
        ]));
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://first-prompt"),
        );
        stepper
            .append_event(event(
                "follow-up",
                12,
                AgentEventKind::Input(InputEvent::FollowUpInputAppended {
                    input_ref: BlobRef::new_unchecked_for_tests("blob://second-prompt"),
                    run_overrides: None,
                }),
            ))
            .expect("follow-up");

        let result = stepper.drive_until_quiescent(32).expect("drive");

        assert_eq!(result.quiescence, StepperQuiescence::Completed);
        assert_eq!(stepper.state.run_history.len(), 2);
        assert!(stepper.state.pending_follow_up_inputs.is_empty());
        assert_eq!(
            stepper.state.run_history[1]
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.output_ref.as_ref()),
            Some(&BlobRef::new_unchecked_for_tests("blob://second"))
        );
    }

    #[test]
    fn local_stepper_handles_context_compaction_prerequisite() {
        let mut stepper = stepper(FakeEffectExecutor::with_llm_receipts([
            LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests(
                    "blob://after-compact",
                )),
                ..Default::default()
            },
        ]));
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://prompt"),
        );
        stepper
            .append_event(event(
                "context-op",
                12,
                AgentEventKind::Lifecycle(LifecycleEvent::ContextOperationStarted {
                    operation: ContextOperationState {
                        operation_id: "compact-1".into(),
                        phase: ContextOperationPhase::Compacting,
                        reason: ContextPressureReason::UsageHighWater,
                        strategy: CompactionStrategy::Summary,
                        ..Default::default()
                    },
                }),
            ))
            .expect("context op");

        let result = stepper.drive_until_quiescent(32).expect("drive");

        assert_eq!(result.quiescence, StepperQuiescence::Completed);
        assert!(
            stepper
                .state
                .context_state
                .pending_context_operation
                .is_none()
        );
        assert!(stepper.state.context_state.last_compaction.is_some());
        assert!(
            stepper
                .projections
                .projection_items
                .iter()
                .any(|item| matches!(item.kind, crate::projection::ProjectionItemKind::Compaction))
        );
    }

    #[test]
    fn interrupt_abandons_pending_effects_and_quiesces() {
        let mut stepper = stepper(FakeEffectExecutor::default());
        open_and_request(
            &mut stepper,
            BlobRef::new_unchecked_for_tests("blob://prompt"),
        );
        let result = stepper.drive_until_quiescent(8).expect("drive pending");
        assert_eq!(result.quiescence, StepperQuiescence::WaitingForEffects);
        assert!(!stepper.state.pending_effects.is_empty());

        stepper
            .append_event(event(
                "interrupt",
                20,
                AgentEventKind::Input(InputEvent::RunInterruptRequested {
                    reason_ref: Some(BlobRef::new_unchecked_for_tests("blob://reason")),
                }),
            ))
            .expect("interrupt");

        assert!(stepper.state.pending_effects.is_empty());
        assert_eq!(
            classify_quiescence(&stepper.state),
            StepperQuiescence::Interrupted
        );
    }
}
