//! Pure decider for turning bounded session state into next loop work.

use crate::batch::ToolCallStatus;
use crate::effects::{
    AgentEffectIntent, AgentEffectKind, LlmCompactRequest, LlmCountTokensRequest,
    LlmGenerationRequest, ToolInvocationRequest,
};
use crate::error::ModelError;
use crate::events::{
    AgentEvent, AgentEventJoins, AgentEventKind, EffectEvent, InputEvent, LifecycleEvent,
};
use crate::ids::{EffectId, IdAllocator, TurnId};
use crate::lifecycle::RunLifecycle;
use crate::planner::{DefaultTurnPlanner, PlannerError, TurnPlanner, TurnPlanningRequest};
use crate::state::{DecideResult, DeciderOutcome, QueuedRunInput, RunState, SessionState};
use crate::tooling::{PlannedToolCall, ToolExecutorKind};
use crate::turn::TurnPrerequisiteKind;
use std::collections::BTreeMap;

static DEFAULT_TURN_PLANNER: DefaultTurnPlanner = DefaultTurnPlanner;

#[derive(Clone, Debug)]
pub struct DecideRequest<'a, P = DefaultTurnPlanner> {
    pub state: &'a SessionState,
    pub planner: &'a P,
    pub observed_at_ms: u64,
    pub stream_llm: bool,
}

impl<'a> DecideRequest<'a, DefaultTurnPlanner> {
    pub fn from_state(state: &'a SessionState) -> Self {
        Self {
            state,
            planner: &DEFAULT_TURN_PLANNER,
            observed_at_ms: state.updated_at_ms,
            stream_llm: false,
        }
    }
}

pub fn decide_next(state: &SessionState) -> DecideResult {
    decide_next_with(DecideRequest::from_state(state))
}

pub fn decide_next_with<P: TurnPlanner>(request: DecideRequest<'_, P>) -> DecideResult {
    let state = request.state;
    let Some(run) = state.current_run.as_ref() else {
        if state.status.accepts_new_runs()
            && let Some(input) = state.pending_follow_up_inputs.front()
        {
            return Ok(DeciderOutcome {
                events: vec![promote_follow_up_event(
                    state,
                    input,
                    request.observed_at_ms,
                )],
                intents: Vec::new(),
            });
        }
        return Ok(DeciderOutcome::default());
    };
    if !state.status.accepts_new_runs() || run.lifecycle.is_terminal() {
        return Ok(DeciderOutcome::default());
    }

    let mut outcome = DeciderOutcome::default();

    if run.lifecycle == RunLifecycle::Queued {
        outcome.events.push(run_lifecycle_event(
            state,
            run,
            RunLifecycle::Queued,
            RunLifecycle::Running,
            request.observed_at_ms,
        ));
        return Ok(outcome);
    }

    if run.lifecycle != RunLifecycle::Running {
        return Ok(outcome);
    }

    if !run.pending_effects.is_empty() || run.active_llm_effect_id.is_some() {
        return Ok(outcome);
    }

    if run
        .outcome
        .as_ref()
        .is_some_and(|outcome| outcome.failure.is_some())
    {
        outcome.events.push(run_lifecycle_event(
            state,
            run,
            RunLifecycle::Running,
            RunLifecycle::Failed,
            request.observed_at_ms,
        ));
        return Ok(outcome);
    }

    if run.latest_output_ref.is_some() {
        push_turn_completed_if_active(state, run, request.observed_at_ms, &mut outcome);
        outcome.events.push(run_lifecycle_event(
            state,
            run,
            RunLifecycle::Running,
            RunLifecycle::Completed,
            request.observed_at_ms,
        ));
        return Ok(outcome);
    }

    if let Some(batch) = run.active_tool_batch.as_ref() {
        let mut ids = state.id_allocator.clone();
        for call_id in batch
            .call_status
            .iter()
            .filter_map(|(call_id, status)| (status == &ToolCallStatus::Queued).then_some(call_id))
        {
            let Some(planned_call) = batch.plan.planned_calls.iter().find(|call| {
                call.accepted
                    && &call.call_id == call_id
                    && !batch.pending_effects.contains_key(&call.call_id)
            }) else {
                continue;
            };
            let intent = tool_intent(
                state,
                run,
                planned_call,
                request.observed_at_ms,
                &mut ids,
                &mut outcome,
            )?;
            outcome.intents.push(intent);
        }
        return Ok(outcome);
    }

    let mut ids = state.id_allocator.clone();
    let replacing_turn = run.active_turn_id.clone();
    let turn_id = ids.allocate_turn_id(&run.run_id);
    enforce_turn_limit(run, &turn_id)?;

    let planned = request
        .planner
        .plan_turn(TurnPlanningRequest::from_state(state, run, turn_id.clone()))
        .map_err(planner_error)?;
    if !planned.plan.is_ready_for_generation() {
        if let Some(prerequisite) = planned.plan.prerequisites.first() {
            let effect_id = ids.allocate_effect_id();
            let kind = match prerequisite.kind {
                TurnPrerequisiteKind::CountTokens => {
                    AgentEffectKind::LlmCountTokens(LlmCountTokensRequest {
                        resolved_context: planned.resolved_context,
                        candidate_plan_id: Some(prerequisite.prerequisite_id.clone()),
                    })
                }
                TurnPrerequisiteKind::CompactContext => {
                    AgentEffectKind::LlmCompact(LlmCompactRequest {
                        resolved_context: planned.resolved_context.clone(),
                        source_items: planned
                            .plan
                            .active_window_items
                            .iter()
                            .map(|item| item.content_ref.clone())
                            .collect(),
                        strategy: state
                            .context_state
                            .pending_context_operation
                            .as_ref()
                            .map(|operation| operation.strategy)
                            .unwrap_or_default(),
                        source_range_start: state
                            .context_state
                            .pending_context_operation
                            .as_ref()
                            .and_then(|operation| operation.source_range.as_ref())
                            .map(|range| range.start_seq),
                        source_range_end: state
                            .context_state
                            .pending_context_operation
                            .as_ref()
                            .and_then(|operation| operation.source_range.as_ref())
                            .map(|range| range.end_seq),
                    })
                }
                TurnPrerequisiteKind::MaterializeToolDefinitions
                | TurnPrerequisiteKind::PrepareToolRuntime
                | TurnPrerequisiteKind::Custom => return Ok(outcome),
            };
            let mut intent = AgentEffectIntent::new(
                effect_id.clone(),
                state.session_id.clone(),
                kind,
                request.observed_at_ms,
            );
            intent.run_id = Some(run.run_id.clone());
            intent.turn_id = Some(turn_id.clone());
            outcome.events.push(effect_intent_event(
                state,
                run,
                Some(&turn_id),
                &effect_id,
                intent.clone(),
                request.observed_at_ms,
            ));
            outcome.intents.push(intent);
        }
        return Ok(outcome);
    }

    if let Some(turn_id) = replacing_turn {
        outcome.events.push(turn_completed_event(
            state,
            run,
            &turn_id,
            request.observed_at_ms,
        ));
    }

    outcome.events.push(turn_started_event(
        state,
        run,
        &turn_id,
        request.observed_at_ms,
    ));

    let effect_id = ids.allocate_effect_id();
    let mut intent = AgentEffectIntent::new(
        effect_id.clone(),
        state.session_id.clone(),
        if request.stream_llm {
            AgentEffectKind::LlmStream(LlmGenerationRequest {
                resolved_context: planned.resolved_context,
                request_ref: None,
                stream: true,
            })
        } else {
            AgentEffectKind::LlmComplete(LlmGenerationRequest {
                resolved_context: planned.resolved_context,
                request_ref: None,
                stream: false,
            })
        },
        request.observed_at_ms,
    );
    intent.run_id = Some(run.run_id.clone());
    intent.turn_id = Some(turn_id.clone());
    outcome.events.push(effect_intent_event(
        state,
        run,
        Some(&turn_id),
        &effect_id,
        intent.clone(),
        request.observed_at_ms,
    ));
    outcome.intents.push(intent);

    Ok(outcome)
}

fn promote_follow_up_event(
    state: &SessionState,
    input: &QueuedRunInput,
    observed_at_ms: u64,
) -> AgentEvent {
    AgentEvent::new(
        format!(
            "promote-follow-up:{}:{}",
            input.queued_at_ms,
            input.input_ref.as_str()
        ),
        state.session_id.clone(),
        observed_at_ms,
        AgentEventKind::Input(InputEvent::RunRequested {
            input_ref: input.input_ref.clone(),
            run_overrides: input.run_overrides.clone(),
        }),
    )
    .with_joins(AgentEventJoins {
        submission_id: input.submission_id.clone(),
        ..Default::default()
    })
}

fn tool_intent(
    state: &SessionState,
    run: &RunState,
    call: &PlannedToolCall,
    observed_at_ms: u64,
    ids: &mut IdAllocator,
    outcome: &mut DeciderOutcome,
) -> Result<AgentEffectIntent, ModelError> {
    let effect_id = ids.allocate_effect_id();
    let mut metadata = BTreeMap::new();
    metadata.insert("tool_name".into(), call.tool_name.clone());
    if let Some(tool_id) = call.tool_id.as_ref() {
        metadata.insert("tool_id".into(), tool_id.clone());
    }

    let mut intent = AgentEffectIntent::new(
        effect_id.clone(),
        state.session_id.clone(),
        AgentEffectKind::ToolInvoke(ToolInvocationRequest {
            call_id: call.call_id.clone(),
            provider_call_id: call.provider_call_id.clone(),
            tool_id: call.tool_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments_json: call.arguments_json.clone(),
            arguments_ref: call.arguments_ref.clone(),
            handler_id: handler_id(&call.executor),
            context_ref: None,
            metadata,
        }),
        observed_at_ms,
    );
    intent.run_id = Some(run.run_id.clone());
    intent.turn_id = run.active_turn_id.clone();

    outcome.events.push(effect_intent_event(
        state,
        run,
        run.active_turn_id.as_ref(),
        &effect_id,
        intent.clone(),
        observed_at_ms,
    ));
    Ok(intent)
}

fn handler_id(executor: &ToolExecutorKind) -> Option<String> {
    match executor {
        ToolExecutorKind::Handler { handler_id } => Some(handler_id.clone()),
        _ => None,
    }
}

fn enforce_turn_limit(run: &RunState, turn_id: &TurnId) -> Result<(), ModelError> {
    let Some(max_turns) = run.config.loop_limits.max_turns else {
        return Ok(());
    };
    let next_run_turn = run.completed_tool_batches.len() as u64 + 1;
    if next_run_turn > max_turns {
        return Err(ModelError::InvalidValue {
            field: "loop_limits.max_turns",
            message: format!(
                "turn '{}' would exceed max_turns {} for run '{}'",
                turn_id, max_turns, run.run_id
            ),
        });
    }
    Ok(())
}

fn planner_error(error: PlannerError) -> ModelError {
    match error {
        PlannerError::EmptySelection => ModelError::InvalidValue {
            field: "turn_plan",
            message: "planner selected no model input refs".into(),
        },
    }
}

fn push_turn_completed_if_active(
    state: &SessionState,
    run: &RunState,
    observed_at_ms: u64,
    outcome: &mut DeciderOutcome,
) {
    if let Some(turn_id) = run.active_turn_id.as_ref() {
        outcome
            .events
            .push(turn_completed_event(state, run, turn_id, observed_at_ms));
    }
}

fn run_lifecycle_event(
    state: &SessionState,
    run: &RunState,
    from: RunLifecycle,
    to: RunLifecycle,
    observed_at_ms: u64,
) -> AgentEvent {
    AgentEvent::new(
        format!("run-lifecycle:{}:{from:?}:{to:?}", run.run_id),
        state.session_id.clone(),
        observed_at_ms,
        AgentEventKind::Lifecycle(LifecycleEvent::RunLifecycleChanged {
            run_id: run.run_id.clone(),
            from,
            to,
        }),
    )
    .with_joins(AgentEventJoins {
        run_id: Some(run.run_id.clone()),
        ..Default::default()
    })
}

fn turn_started_event(
    state: &SessionState,
    run: &RunState,
    turn_id: &TurnId,
    observed_at_ms: u64,
) -> AgentEvent {
    AgentEvent::new(
        format!("turn-started:{turn_id}"),
        state.session_id.clone(),
        observed_at_ms,
        AgentEventKind::Lifecycle(LifecycleEvent::TurnStarted {
            turn_id: turn_id.clone(),
        }),
    )
    .with_joins(AgentEventJoins {
        run_id: Some(run.run_id.clone()),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    })
}

fn turn_completed_event(
    state: &SessionState,
    run: &RunState,
    turn_id: &TurnId,
    observed_at_ms: u64,
) -> AgentEvent {
    AgentEvent::new(
        format!("turn-completed:{turn_id}"),
        state.session_id.clone(),
        observed_at_ms,
        AgentEventKind::Lifecycle(LifecycleEvent::TurnCompleted {
            turn_id: turn_id.clone(),
        }),
    )
    .with_joins(AgentEventJoins {
        run_id: Some(run.run_id.clone()),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    })
}

fn effect_intent_event(
    state: &SessionState,
    run: &RunState,
    turn_id: Option<&TurnId>,
    effect_id: &EffectId,
    intent: AgentEffectIntent,
    observed_at_ms: u64,
) -> AgentEvent {
    AgentEvent::new(
        format!("effect-intent:{effect_id}"),
        state.session_id.clone(),
        observed_at_ms,
        AgentEventKind::Effect(EffectEvent::EffectIntentRecorded { intent }),
    )
    .with_joins(AgentEventJoins {
        run_id: Some(run.run_id.clone()),
        turn_id: turn_id.cloned(),
        effect_id: Some(effect_id.clone()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{AgentReceiptKind, LlmGenerationReceipt, ToolInvocationReceipt};
    use crate::events::{InputEvent, ToolOverrides};
    use crate::ids::{JournalSeq, SessionId, ToolCallId};
    use crate::journal::InMemoryJournal;
    use crate::reducer::apply_event;
    use crate::refs::BlobRef;
    use crate::state::SessionState;
    use crate::tooling::{ToolCallObserved, ToolProfile, ToolRegistry, ToolSpec};
    use serde_json::json;

    fn state_and_journal() -> (SessionState, InMemoryJournal) {
        let state = SessionState::new(SessionId::new("session-a"), Default::default(), 1);
        let journal = InMemoryJournal::new(state.session_id.clone());
        (state, journal)
    }

    fn append_apply(state: &mut SessionState, journal: &mut InMemoryJournal, event: AgentEvent) {
        let appended = journal.append(event).expect("append event");
        apply_event(state, &appended.event).expect("reduce event");
    }

    fn append_apply_all(
        state: &mut SessionState,
        journal: &mut InMemoryJournal,
        events: Vec<AgentEvent>,
    ) {
        for event in events {
            append_apply(state, journal, event);
        }
    }

    fn open_and_request_run(state: &mut SessionState, journal: &mut InMemoryJournal) {
        append_apply(
            state,
            journal,
            AgentEvent::new(
                "open",
                state.session_id.clone(),
                10,
                AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
            ),
        );
        append_apply(
            state,
            journal,
            AgentEvent::new(
                "run-requested",
                state.session_id.clone(),
                11,
                AgentEventKind::Input(InputEvent::RunRequested {
                    input_ref: BlobRef::new_unchecked_for_tests("blob://prompt"),
                    run_overrides: None,
                }),
            ),
        );
    }

    fn install_echo_tool(state: &mut SessionState, journal: &mut InMemoryJournal) {
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
        append_apply(
            state,
            journal,
            AgentEvent::new(
                "tool-registry",
                state.session_id.clone(),
                12,
                AgentEventKind::Input(InputEvent::ToolRegistrySet { registry }),
            ),
        );
        append_apply(
            state,
            journal,
            AgentEvent::new(
                "tool-profile",
                state.session_id.clone(),
                13,
                AgentEventKind::Input(InputEvent::ToolProfileSelected {
                    profile_id: "local".into(),
                }),
            ),
        );
    }

    fn receipt_event(intent: &AgentEffectIntent, kind: AgentReceiptKind, at_ms: u64) -> AgentEvent {
        let receipt = intent.receipt(kind, at_ms);
        AgentEvent::new(
            format!("receipt:{}", receipt.effect_id),
            receipt.session_id.clone(),
            at_ms,
            AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }),
        )
        .with_joins(AgentEventJoins {
            run_id: intent.run_id.clone(),
            turn_id: intent.turn_id.clone(),
            effect_id: Some(intent.effect_id.clone()),
            ..Default::default()
        })
    }

    fn start_queued_run(state: &mut SessionState, journal: &mut InMemoryJournal) {
        let outcome = decide_next(state).expect("decide start");
        assert_eq!(outcome.events.len(), 1);
        assert!(outcome.intents.is_empty());
        append_apply_all(state, journal, outcome.events);
        assert_eq!(
            state.current_run.as_ref().expect("run").lifecycle,
            RunLifecycle::Running
        );
    }

    fn emit_initial_llm(
        state: &mut SessionState,
        journal: &mut InMemoryJournal,
    ) -> AgentEffectIntent {
        let outcome = decide_next(state).expect("decide llm");
        assert_eq!(outcome.events.len(), 2);
        assert_eq!(outcome.intents.len(), 1);
        let intent = outcome.intents[0].clone();
        append_apply_all(state, journal, outcome.events);
        assert_eq!(
            state
                .current_run
                .as_ref()
                .expect("run")
                .active_llm_effect_id,
            Some(intent.effect_id.clone())
        );
        intent
    }

    #[test]
    fn decide_next_completes_fake_llm_answer_run() {
        let (mut state, mut journal) = state_and_journal();
        open_and_request_run(&mut state, &mut journal);
        start_queued_run(&mut state, &mut journal);

        let llm_intent = emit_initial_llm(&mut state, &mut journal);
        append_apply(
            &mut state,
            &mut journal,
            receipt_event(
                &llm_intent,
                AgentReceiptKind::LlmComplete(LlmGenerationReceipt {
                    assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://answer")),
                    ..Default::default()
                }),
                30,
            ),
        );

        let outcome = decide_next(&state).expect("decide completion");
        assert_eq!(outcome.events.len(), 2);
        append_apply_all(&mut state, &mut journal, outcome.events);

        assert!(state.current_run.is_none());
        assert_eq!(state.run_history.len(), 1);
        assert_eq!(
            state.run_history[0]
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.output_ref.as_ref()),
            Some(&BlobRef::new_unchecked_for_tests("blob://answer"))
        );
        assert_eq!(journal.latest_seq(), Some(JournalSeq(8)));
    }

    #[test]
    fn decide_next_continues_after_fake_tool_round_trip() {
        let (mut state, mut journal) = state_and_journal();
        open_and_request_run(&mut state, &mut journal);
        install_echo_tool(&mut state, &mut journal);
        let session_id = state.session_id.clone();
        append_apply(
            &mut state,
            &mut journal,
            AgentEvent::new(
                "tool-overrides",
                session_id,
                14,
                AgentEventKind::Input(InputEvent::ToolOverridesSet {
                    overrides: ToolOverrides {
                        enable: vec!["echo".into()],
                        ..Default::default()
                    },
                }),
            ),
        );
        start_queued_run(&mut state, &mut journal);

        let llm_intent = emit_initial_llm(&mut state, &mut journal);
        append_apply(
            &mut state,
            &mut journal,
            receipt_event(
                &llm_intent,
                AgentReceiptKind::LlmComplete(LlmGenerationReceipt {
                    raw_provider_response_ref: Some(BlobRef::new_unchecked_for_tests(
                        "blob://raw-tool-call",
                    )),
                    tool_calls: vec![ToolCallObserved {
                        call_id: ToolCallId::new("call-1"),
                        tool_name: "echo".into(),
                        arguments_json: Some(r#"{"text":"hi"}"#.into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                30,
            ),
        );

        let tool_outcome = decide_next(&state).expect("decide tool");
        assert_eq!(tool_outcome.events.len(), 1);
        assert_eq!(tool_outcome.intents.len(), 1);
        let tool_intent = tool_outcome.intents[0].clone();
        append_apply_all(&mut state, &mut journal, tool_outcome.events);

        append_apply(
            &mut state,
            &mut journal,
            receipt_event(
                &tool_intent,
                AgentReceiptKind::ToolInvoke(ToolInvocationReceipt {
                    call_id: ToolCallId::new("call-1"),
                    tool_id: Some("echo".into()),
                    tool_name: "echo".into(),
                    output_ref: Some(BlobRef::new_unchecked_for_tests("blob://tool-output")),
                    model_visible_output_ref: Some(BlobRef::new_unchecked_for_tests(
                        "blob://tool-visible",
                    )),
                    is_error: false,
                    metadata: BTreeMap::new(),
                }),
                40,
            ),
        );

        let continuation = decide_next(&state).expect("decide continuation");
        assert_eq!(continuation.events.len(), 3);
        assert_eq!(continuation.intents.len(), 1);
        let second_llm = continuation.intents[0].clone();
        let AgentEffectKind::LlmComplete(request) = &second_llm.kind else {
            panic!("expected llm completion intent");
        };
        assert!(request.resolved_context.active_window_items.iter().any(
            |item| item.content_ref == BlobRef::new_unchecked_for_tests("blob://tool-visible")
        ));
        append_apply_all(&mut state, &mut journal, continuation.events);

        append_apply(
            &mut state,
            &mut journal,
            receipt_event(
                &second_llm,
                AgentReceiptKind::LlmComplete(LlmGenerationReceipt {
                    assistant_message_ref: Some(BlobRef::new_unchecked_for_tests(
                        "blob://final-answer",
                    )),
                    ..Default::default()
                }),
                50,
            ),
        );
        let completion = decide_next(&state).expect("decide final completion");
        append_apply_all(&mut state, &mut journal, completion.events);

        assert!(state.current_run.is_none());
        assert_eq!(
            state.run_history[0]
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.output_ref.as_ref()),
            Some(&BlobRef::new_unchecked_for_tests("blob://final-answer"))
        );
    }
}
