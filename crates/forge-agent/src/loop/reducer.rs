//! Pure reducer for applying ordered journal events to bounded session state.

use crate::batch::{ActiveToolBatch, PendingToolEffect, ToolCallModelResult, ToolCallStatus};
use crate::config::{RunConfig, SessionConfig};
use crate::context::{ActiveWindowItem, CompactionBlobKind, CompactionRecord, ContextInputLane};
use crate::effects::{AgentEffectIntent, AgentEffectKind, AgentReceiptKind, ToolInvocationReceipt};
use crate::error::ModelError;
use crate::events::{
    AgentEvent, AgentEventKind, EffectEvent, InputEvent, LifecycleEvent, ToolOverrideScope,
};
use crate::ids::{EffectId, JournalSeq, RunId, ToolCallId, TurnId};
use crate::lifecycle::{RunLifecycle, SessionStatus, TurnLifecycle};
use crate::refs::BlobRef;
use crate::state::{
    PendingEffectRecord, PendingEffectStatus, QueuedRunInput, QueuedSteeringInput, ReduceResult,
    ReducerOutcome, RunCause, RunOutcome, RunState, SessionState,
};
use crate::tooling::{PlannedToolCall, ToolBatchPlan, ToolCallObserved, ToolProfile};
use std::collections::{BTreeMap, BTreeSet};

pub fn apply_event(state: &mut SessionState, event: &AgentEvent) -> ReduceResult {
    validate_event_order(state, event)?;

    match &event.kind {
        AgentEventKind::Input(input) => apply_input_event(state, event, input)?,
        AgentEventKind::Lifecycle(lifecycle) => apply_lifecycle_event(state, event, lifecycle)?,
        AgentEventKind::Effect(effect) => apply_effect_event(state, effect)?,
        AgentEventKind::Observation(_) => {}
    }

    state.latest_journal_seq = event.journal_seq;
    state.updated_at_ms = event.observed_at_ms;

    Ok(ReducerOutcome::default())
}

fn apply_effect_event(state: &mut SessionState, effect: &EffectEvent) -> Result<(), ModelError> {
    match effect {
        EffectEvent::EffectIntentRecorded { intent } => record_effect_intent(state, intent)?,
        EffectEvent::EffectStreamFrameObserved { frame } => {
            mark_pending_effect_streaming(state, &frame.effect_id);
        }
        EffectEvent::EffectReceiptRecorded { receipt } => {
            let pending = state.pending_effects.get(&receipt.effect_id).cloned();
            settle_pending_effect(state, &receipt.effect_id);
            if let Some(run) = state.current_run.as_mut()
                && receipt.run_id.as_ref() == Some(&run.run_id)
            {
                let completed_at_ms = receipt.completed_at_ms;
                match &receipt.kind {
                    AgentReceiptKind::LlmComplete(receipt)
                    | AgentReceiptKind::LlmStream(receipt) => {
                        if let Some(usage) = receipt.usage {
                            run.usage_records.push(usage);
                            state.context_state.last_llm_usage = Some(usage);
                        }
                        if receipt.assistant_message_ref.is_some() && receipt.tool_calls.is_empty()
                        {
                            run.latest_output_ref = receipt.assistant_message_ref.clone();
                        }
                        if !receipt.tool_calls.is_empty() {
                            let run_id = run.run_id.clone();
                            let source_output_ref = receipt
                                .assistant_message_ref
                                .clone()
                                .or_else(|| receipt.raw_provider_response_ref.clone());
                            let tool_calls = receipt.tool_calls.clone();
                            let _ = run;
                            create_active_tool_batch(
                                state,
                                &run_id,
                                effect.effect_id().clone(),
                                source_output_ref,
                                tool_calls,
                            )?;
                        }
                    }
                    AgentReceiptKind::LlmCountTokens(receipt) => {
                        state.context_state.last_token_count = Some(receipt.token_count.clone());
                        state.context_state.clear_pending_operation();
                    }
                    AgentReceiptKind::LlmCompact(receipt) => {
                        if let Some(record) = compaction_record_from_pending(
                            pending.as_ref(),
                            receipt.blob_refs.clone(),
                            receipt.warnings.clone(),
                            receipt.usage,
                            completed_at_ms,
                        ) {
                            state.context_state.apply_compaction(record);
                        } else {
                            state.context_state.clear_pending_operation();
                        }
                    }
                    AgentReceiptKind::ToolInvoke(receipt) => {
                        apply_tool_receipt(state, receipt);
                    }
                    AgentReceiptKind::Failed(failure) if !failure.retryable => {
                        run.outcome = Some(RunOutcome::failed(
                            failure.code.clone(),
                            failure.detail.clone(),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn create_active_tool_batch(
    state: &mut SessionState,
    run_id: &RunId,
    source_effect_id: EffectId,
    source_output_ref: Option<BlobRef>,
    observed_calls: Vec<ToolCallObserved>,
) -> Result<(), ModelError> {
    let profile_id = state
        .current_run
        .as_ref()
        .and_then(|run| run.config.tool_profile.as_ref())
        .or(state.selected_tool_profile.as_ref())
        .or(state.config.default_tool_profile.as_ref())
        .cloned();
    let selected_tools = selected_tools_by_model_name(state, profile_id.as_deref());
    let disabled = disabled_tool_ids_for_run(state, run_id);
    let enabled = enabled_tool_ids_for_run(state, run_id);

    let planned_calls = observed_calls
        .iter()
        .map(|call| {
            let Some(tool) = selected_tools.get(&call.tool_name) else {
                return PlannedToolCall::unavailable(call, "unknown or unavailable tool");
            };
            if disabled.contains(&tool.tool_id) {
                return PlannedToolCall::unavailable(call, "tool disabled by profile or override");
            }
            if !enabled.is_empty() && !enabled.contains(&tool.tool_id) {
                return PlannedToolCall::unavailable(call, "tool not enabled by override");
            }
            PlannedToolCall::accepted(call, tool)
        })
        .collect::<Vec<_>>();

    let plan = ToolBatchPlan::from_planned_calls(observed_calls, planned_calls);
    let batch_id = state.id_allocator.allocate_tool_batch_id(run_id);
    let mut batch = ActiveToolBatch::new(batch_id, source_effect_id, source_output_ref, plan);
    mark_unavailable_tool_results(&mut batch);
    complete_batch_if_settled(state, batch);
    Ok(())
}

fn selected_tools_by_model_name(
    state: &SessionState,
    profile_id: Option<&str>,
) -> BTreeMap<String, crate::tooling::ToolSpec> {
    let tools = if let Some(profile_id) = profile_id {
        state
            .tool_registry
            .model_visible_tools_for_profile(profile_id)
    } else {
        state.tool_registry.tools_by_id.values().cloned().collect()
    };
    tools
        .into_iter()
        .map(|tool| (tool.tool_name.clone(), tool))
        .collect()
}

fn disabled_tool_ids_for_run(state: &SessionState, run_id: &RunId) -> BTreeSet<String> {
    let mut disabled = state
        .config
        .default_tool_disable
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(profile_id) = state
        .current_run
        .as_ref()
        .and_then(|run| run.config.tool_profile.as_ref())
        .or(state.selected_tool_profile.as_ref())
        .or(state.config.default_tool_profile.as_ref())
        && let Some(profile) = state.tool_registry.profiles.get(profile_id)
    {
        disabled.extend(profile.disabled_tool_ids.iter().cloned());
    }
    if let Some(run) = state
        .current_run
        .as_ref()
        .filter(|run| &run.run_id == run_id)
    {
        disabled.extend(run.config.tool_disable.iter().cloned());
    }
    disabled
}

fn enabled_tool_ids_for_run(state: &SessionState, run_id: &RunId) -> BTreeSet<String> {
    let mut enabled = state
        .config
        .default_tool_enable
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(run) = state
        .current_run
        .as_ref()
        .filter(|run| &run.run_id == run_id)
    {
        enabled.extend(run.config.tool_enable.iter().cloned());
    }
    enabled
}

fn mark_unavailable_tool_results(batch: &mut ActiveToolBatch) {
    let unavailable = batch
        .plan
        .planned_calls
        .iter()
        .filter(|call| !call.accepted)
        .cloned()
        .collect::<Vec<_>>();
    for call in unavailable {
        let detail = call
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| "tool unavailable".into());
        batch.set_call_status(
            call.call_id.clone(),
            ToolCallStatus::Failed {
                code: "tool_unavailable".into(),
                detail: detail.clone(),
            },
        );
        batch.model_results.insert(
            call.call_id.clone(),
            ToolCallModelResult {
                call_id: call.call_id.clone(),
                tool_id: call.tool_id.clone(),
                tool_name: call.tool_name.clone(),
                is_error: true,
                output_ref: synthetic_tool_result_ref(&call.call_id, &detail),
                model_visible_output_ref: None,
            },
        );
    }
}

fn synthetic_tool_result_ref(call_id: &ToolCallId, preview: &str) -> BlobRef {
    let payload = format!("{call_id}:{preview}");
    BlobRef::from_bytes(payload.as_bytes())
}

fn apply_tool_receipt(state: &mut SessionState, receipt: &ToolInvocationReceipt) {
    let Some(run) = state.current_run.as_mut() else {
        return;
    };
    let Some(mut batch) = run.active_tool_batch.take() else {
        return;
    };
    if !batch.contains_call(&receipt.call_id) {
        run.active_tool_batch = Some(batch);
        return;
    }

    let status = if receipt.is_error {
        ToolCallStatus::Failed {
            code: "tool_error".into(),
            detail: receipt.tool_name.clone(),
        }
    } else {
        ToolCallStatus::Succeeded
    };
    batch.set_call_status(receipt.call_id.clone(), status);
    batch.pending_effects.remove(&receipt.call_id);
    batch.model_results.insert(
        receipt.call_id.clone(),
        ToolCallModelResult {
            call_id: receipt.call_id.clone(),
            tool_id: receipt.tool_id.clone(),
            tool_name: receipt.tool_name.clone(),
            is_error: receipt.is_error,
            output_ref: receipt
                .output_ref
                .clone()
                .unwrap_or_else(|| synthetic_tool_result_ref(&receipt.call_id, "")),
            model_visible_output_ref: receipt.model_visible_output_ref.clone(),
        },
    );

    if batch.is_settled() {
        run.completed_tool_batches.push(batch);
    } else {
        run.active_tool_batch = Some(batch);
    }
}

fn complete_batch_if_settled(state: &mut SessionState, batch: ActiveToolBatch) {
    let Some(run) = state.current_run.as_mut() else {
        return;
    };
    if batch.is_settled() {
        run.completed_tool_batches.push(batch);
    } else {
        run.active_tool_batch = Some(batch);
    }
}

fn compaction_record_from_pending(
    pending: Option<&PendingEffectRecord>,
    blob_refs: Vec<BlobRef>,
    warnings: Vec<String>,
    usage: Option<crate::context::LlmUsageRecord>,
    created_at_ms: u64,
) -> Option<CompactionRecord> {
    let Some(PendingEffectRecord {
        intent:
            AgentEffectIntent {
                kind: AgentEffectKind::LlmCompact(request),
                ..
            },
        ..
    }) = pending
    else {
        return None;
    };

    let first_blob = blob_refs.first()?.clone();
    let source_range = crate::transcript::TranscriptRange {
        start_seq: request.source_range_start.unwrap_or_default(),
        end_seq: request.source_range_end.unwrap_or_else(|| {
            request
                .source_range_start
                .unwrap_or_default()
                .saturating_add(request.source_items.len() as u64)
        }),
    };

    Some(CompactionRecord {
        operation_id: request
            .resolved_context
            .turn_id
            .to_string()
            .replace(':', "-"),
        strategy: request.strategy,
        blob_kind: CompactionBlobKind::Summary,
        blob_refs: blob_refs.clone(),
        source_range: source_range.clone(),
        source_refs: request.source_items.clone(),
        active_window_items: vec![ActiveWindowItem::message_ref(
            "compaction-summary",
            first_blob,
            Some(ContextInputLane::Summary),
            Some(source_range),
        )],
        provider_compatibility: None,
        usage,
        created_at_ms,
        warnings,
    })
}

fn record_effect_intent(
    state: &mut SessionState,
    intent: &AgentEffectIntent,
) -> Result<(), ModelError> {
    if state.pending_effects.contains_key(&intent.effect_id) {
        return Err(ModelError::InvalidValue {
            field: "effect_id",
            message: format!("effect '{}' is already pending", intent.effect_id),
        });
    }

    if let Some(run_id) = intent.run_id.as_ref() {
        let Some(run) = state.current_run.as_ref() else {
            return Err(ModelError::InvalidValue {
                field: "current_run",
                message: "run-scoped effect intent requires an active run".into(),
            });
        };
        if &run.run_id != run_id {
            return Err(ModelError::InvalidValue {
                field: "run_id",
                message: "effect intent run does not match current run".into(),
            });
        }
    }

    let record = PendingEffectRecord {
        intent: intent.clone(),
        status: PendingEffectStatus::Pending,
    };
    advance_effect_allocator(state, &intent.effect_id);
    state
        .pending_effects
        .insert(intent.effect_id.clone(), record.clone());

    if let Some(run_id) = intent.run_id.as_ref() {
        let run = state.current_run.as_mut().expect("validated current run");
        debug_assert_eq!(&run.run_id, run_id);
        run.pending_effects.insert(intent.effect_id.clone(), record);
        if matches!(
            intent.kind,
            crate::effects::AgentEffectKind::LlmComplete(_)
                | crate::effects::AgentEffectKind::LlmStream(_)
        ) {
            run.active_llm_effect_id = Some(intent.effect_id.clone());
        }
        if let crate::effects::AgentEffectKind::ToolInvoke(request) = &intent.kind
            && let Some(batch) = run.active_tool_batch.as_mut()
            && batch.contains_call(&request.call_id)
        {
            batch.set_call_status(request.call_id.clone(), ToolCallStatus::Pending);
            batch.pending_effects.insert(
                request.call_id.clone(),
                PendingToolEffect {
                    call_id: request.call_id.clone(),
                    effect_id: intent.effect_id.clone(),
                    emitted_at_ms: intent.emitted_at_ms,
                },
            );
        }
    }

    Ok(())
}

fn mark_pending_effect_streaming(state: &mut SessionState, effect_id: &EffectId) {
    if let Some(record) = state.pending_effects.get_mut(effect_id)
        && !record.status.is_terminal()
    {
        record.status = PendingEffectStatus::Streaming;
    }
    if let Some(run) = state.current_run.as_mut()
        && let Some(record) = run.pending_effects.get_mut(effect_id)
        && !record.status.is_terminal()
    {
        record.status = PendingEffectStatus::Streaming;
    }
}

fn advance_effect_allocator(state: &mut SessionState, effect_id: &EffectId) {
    if effect_id.session_id == state.session_id {
        state.id_allocator.next_effect_seq = state
            .id_allocator
            .next_effect_seq
            .max(effect_id.effect_seq.saturating_add(1));
    }
}

fn advance_turn_allocator(state: &mut SessionState, turn_id: &TurnId) {
    if turn_id.run_id.session_id == state.session_id {
        state.id_allocator.next_turn_seq = state
            .id_allocator
            .next_turn_seq
            .max(turn_id.turn_seq.saturating_add(1));
    }
}

fn settle_pending_effect(state: &mut SessionState, effect_id: &EffectId) {
    state.pending_effects.remove(effect_id);
    if let Some(run) = state.current_run.as_mut() {
        run.pending_effects.remove(effect_id);
        if run.active_llm_effect_id.as_ref() == Some(effect_id) {
            run.active_llm_effect_id = None;
        }
    }
}

pub fn apply_events<'a>(
    state: &mut SessionState,
    events: impl IntoIterator<Item = &'a AgentEvent>,
) -> ReduceResult {
    let mut outcome = ReducerOutcome::default();
    for event in events {
        let event_outcome = apply_event(state, event)?;
        outcome.emitted_events.extend(event_outcome.emitted_events);
    }
    Ok(outcome)
}

fn validate_event_order(state: &SessionState, event: &AgentEvent) -> Result<(), ModelError> {
    if event.session_id != state.session_id {
        return Err(ModelError::InvalidValue {
            field: "session_id",
            message: format!(
                "state for session '{}' cannot reduce event for session '{}'",
                state.session_id, event.session_id
            ),
        });
    }

    let Some(actual) = event.journal_seq else {
        return Err(ModelError::InvalidValue {
            field: "journal_seq",
            message: "reducer requires an assigned journal sequence".into(),
        });
    };

    let expected = next_expected_seq(state.latest_journal_seq);
    if actual != expected {
        return Err(ModelError::InvalidValue {
            field: "journal_seq",
            message: format!("expected sequence {}, got {}", expected, actual),
        });
    }

    Ok(())
}

fn next_expected_seq(latest: Option<JournalSeq>) -> JournalSeq {
    JournalSeq(latest.map_or(1, |seq| seq.0.saturating_add(1)))
}

fn apply_input_event(
    state: &mut SessionState,
    event: &AgentEvent,
    input: &InputEvent,
) -> Result<(), ModelError> {
    match input {
        InputEvent::SessionOpened { config } => {
            if let Some(config) = config {
                apply_session_config_without_boundary(state, config.clone());
            }
            state.transition_status(SessionStatus::Active, event.observed_at_ms)?;
        }
        InputEvent::SessionConfigUpdated { config } => {
            apply_session_config_boundary(state, config.clone());
        }
        InputEvent::RunRequested {
            input_ref,
            run_overrides,
        } => {
            request_or_queue_run(
                state,
                event,
                QueuedRunInput {
                    submission_id: event.joins.submission_id.clone(),
                    input_ref: input_ref.clone(),
                    run_overrides: run_overrides.clone(),
                    queued_at_ms: event.observed_at_ms,
                },
            );
        }
        InputEvent::FollowUpInputAppended {
            input_ref,
            run_overrides,
        } => {
            state.enqueue_follow_up(QueuedRunInput {
                submission_id: event.joins.submission_id.clone(),
                input_ref: input_ref.clone(),
                run_overrides: run_overrides.clone(),
                queued_at_ms: event.observed_at_ms,
            });
        }
        InputEvent::RunSteerRequested { instruction_ref } => {
            state.enqueue_steering(QueuedSteeringInput {
                instruction_ref: instruction_ref.clone(),
                queued_at_ms: event.observed_at_ms,
            });
        }
        InputEvent::RunInterruptRequested { reason_ref } => {
            abandon_pending_effects(state);
            state.finish_current_run(
                RunLifecycle::Interrupted,
                RunOutcome {
                    interrupted_reason_ref: reason_ref.clone(),
                    ..Default::default()
                },
                event.observed_at_ms,
            )?;
        }
        InputEvent::SessionPaused => {
            state.transition_status(SessionStatus::Paused, event.observed_at_ms)?;
        }
        InputEvent::SessionResumed => {
            state.transition_status(SessionStatus::Active, event.observed_at_ms)?;
        }
        InputEvent::SessionClosed => {
            state.transition_status(SessionStatus::Closed, event.observed_at_ms)?;
        }
        InputEvent::TurnContextOverrideRequested { turn_id, .. } => {
            if let Some(turn_id) = turn_id {
                validate_turn_belongs_to_current_run(state, turn_id)?;
            }
        }
        InputEvent::SessionHistoryRewriteRequested { request } => {
            state.apply_history_rewrite(
                request.rewrite_id.clone(),
                request.replacement_boundary.clone(),
                request.replacement_blob_refs.first().cloned(),
            );
        }
        InputEvent::SessionHistoryRollbackRequested { request } => {
            state.apply_history_rollback(
                request.rollback_id.clone(),
                None,
                request.reason_ref.clone(),
            );
        }
        InputEvent::ToolRegistrySet { registry } => {
            state.tool_registry = registry.clone();
            if state
                .selected_tool_profile
                .as_ref()
                .is_some_and(|profile_id| !state.tool_registry.profiles.contains_key(profile_id))
            {
                state.selected_tool_profile = None;
            }
            bump_config_revision(state);
        }
        InputEvent::ToolProfileSelected { profile_id } => {
            if !state.tool_registry.profiles.contains_key(profile_id) {
                return Err(ModelError::InvalidValue {
                    field: "profile_id",
                    message: format!("unknown tool profile '{}'", profile_id),
                });
            }
            state.selected_tool_profile = Some(profile_id.clone());
            state.config.default_tool_profile = Some(profile_id.clone());
            bump_config_revision(state);
        }
        InputEvent::ToolOverridesSet { overrides } => {
            apply_tool_overrides(
                state,
                overrides.scope,
                overrides.profile.clone(),
                &overrides.enable,
                &overrides.disable,
                &overrides.force,
            )?;
        }
        InputEvent::ConfirmationProvided { request_id, .. } => {
            if state
                .pending_confirmation_requests
                .remove(request_id)
                .is_none()
            {
                return Err(ModelError::InvalidValue {
                    field: "request_id",
                    message: format!("unknown confirmation request '{}'", request_id),
                });
            }
        }
    }

    Ok(())
}

fn apply_lifecycle_event(
    state: &mut SessionState,
    event: &AgentEvent,
    lifecycle: &LifecycleEvent,
) -> Result<(), ModelError> {
    match lifecycle {
        LifecycleEvent::SessionLifecycleChanged { from, to }
        | LifecycleEvent::SessionStatusChanged { from, to } => {
            ensure_session_status(state, *from)?;
            state.transition_status(*to, event.observed_at_ms)?;
        }
        LifecycleEvent::RunLifecycleChanged { run_id, from, to } => {
            apply_run_lifecycle_changed(state, run_id, *from, *to, event.observed_at_ms)?;
        }
        LifecycleEvent::TurnStarted { turn_id } => {
            let run = current_run_for_turn_mut(state, turn_id)?;
            if run
                .active_turn_id
                .as_ref()
                .is_some_and(|active| active != turn_id)
            {
                return Err(ModelError::InvalidValue {
                    field: "active_turn_id",
                    message: "another turn is already active".into(),
                });
            }
            run.active_turn_id = Some(turn_id.clone());
            run.updated_at_ms = event.observed_at_ms;
            advance_turn_allocator(state, turn_id);
        }
        LifecycleEvent::TurnCompleted { turn_id } => {
            clear_active_turn(state, turn_id, event.observed_at_ms)?;
        }
        LifecycleEvent::TurnFailed { turn_id, .. } => {
            clear_active_turn(state, turn_id, event.observed_at_ms)?;
        }
        LifecycleEvent::TurnLifecycleChanged { turn_id, from, to } => {
            validate_turn_belongs_to_current_run(state, turn_id)?;
            validate_turn_transition(*from, *to)?;
            if to.is_terminal() {
                clear_active_turn(state, turn_id, event.observed_at_ms)?;
            }
        }
        LifecycleEvent::ContextOperationStarted { operation } => {
            state.context_state.set_pending_operation(operation.clone());
        }
        LifecycleEvent::ContextOperationCompleted { operation } => {
            if state
                .context_state
                .pending_context_operation
                .as_ref()
                .is_some_and(|pending| pending.operation_id == operation.operation_id)
            {
                state.context_state.clear_pending_operation();
            }
        }
        LifecycleEvent::ContextPressureRecorded { pressure } => {
            state.context_state.last_context_pressure = Some(pressure.clone());
        }
        LifecycleEvent::HistoryRewriteCompleted {
            rewrite_id,
            resulting_boundary,
        } => {
            if state.history.latest_rewrite_id.as_ref() != Some(rewrite_id) {
                state.apply_history_rewrite(rewrite_id.clone(), resulting_boundary.clone(), None);
            } else if resulting_boundary.is_some() {
                state.history.active_boundary = resulting_boundary.clone();
            }
        }
        LifecycleEvent::ToolBatchStarted { .. } | LifecycleEvent::ToolBatchCompleted { .. } => {}
    }

    Ok(())
}

fn apply_session_config_without_boundary(state: &mut SessionState, config: SessionConfig) {
    state.effective_agent_version_id = config.initial_agent_version_id.clone();
    state.selected_tool_profile = config.default_tool_profile.clone();
    state.config = config;
}

fn apply_session_config_boundary(state: &mut SessionState, config: SessionConfig) {
    apply_session_config_without_boundary(state, config);
    bump_config_revision(state);
}

fn bump_config_revision(state: &mut SessionState) {
    state.config_revision = state.config_revision.saturating_add(1);
    state.effective_agent_version_id = state.config.initial_agent_version_id.clone();
}

fn request_or_queue_run(state: &mut SessionState, event: &AgentEvent, input: QueuedRunInput) {
    if state.current_run.is_none() && state.status.accepts_new_runs() {
        if state
            .pending_follow_up_inputs
            .front()
            .is_some_and(|queued| queued_run_input_matches(queued, &input))
        {
            state.pending_follow_up_inputs.pop_front();
        }
        let run_id = state.id_allocator.allocate_run_id();
        let cause = RunCause::direct_input(input.input_ref.clone(), input.submission_id.clone());
        let run = RunState::queued(
            run_id,
            cause,
            state.effective_agent_version_id.clone(),
            state.config_revision,
            RunConfig::from_session(&state.config, input.run_overrides.as_ref()),
            event.observed_at_ms,
        );
        state.current_run = Some(run);
    } else {
        state.enqueue_follow_up(input);
    }
}

fn queued_run_input_matches(left: &QueuedRunInput, right: &QueuedRunInput) -> bool {
    left.submission_id == right.submission_id
        && left.input_ref == right.input_ref
        && left.run_overrides == right.run_overrides
}

fn abandon_pending_effects(state: &mut SessionState) {
    for record in state.pending_effects.values_mut() {
        record.status = PendingEffectStatus::Abandoned;
    }
    state.pending_effects.clear();
    if let Some(run) = state.current_run.as_mut() {
        for record in run.pending_effects.values_mut() {
            record.status = PendingEffectStatus::Abandoned;
        }
        run.pending_effects.clear();
        run.active_llm_effect_id = None;
        if let Some(batch) = run.active_tool_batch.as_mut() {
            let pending_call_ids = batch
                .pending_effects
                .values()
                .map(|pending| pending.call_id.clone())
                .collect::<Vec<_>>();
            for call_id in pending_call_ids {
                batch.set_call_status(call_id, ToolCallStatus::Cancelled);
            }
            batch.pending_effects.clear();
        }
    }
}

fn apply_tool_overrides(
    state: &mut SessionState,
    scope: ToolOverrideScope,
    profile: Option<ToolProfile>,
    enable: &[String],
    disable: &[String],
    force: &[String],
) -> Result<(), ModelError> {
    match scope {
        ToolOverrideScope::Session => {
            if let Some(profile) = profile {
                state.selected_tool_profile = Some(profile.profile_id.clone());
                state.config.default_tool_profile = Some(profile.profile_id.clone());
                state.tool_registry.insert_profile(profile);
            }
            state.config.default_tool_enable = enable.to_vec();
            state.config.default_tool_disable = disable.to_vec();
            state.config.default_tool_force = force.to_vec();
            bump_config_revision(state);
        }
        ToolOverrideScope::Run => {
            let Some(run) = state.current_run.as_mut() else {
                return Err(ModelError::InvalidValue {
                    field: "current_run",
                    message: "run-scoped tool overrides require an active or queued run".into(),
                });
            };
            if let Some(profile) = profile {
                run.config.tool_profile = Some(profile.profile_id.clone());
                state.tool_registry.insert_profile(profile);
            }
            run.config.tool_enable = enable.to_vec();
            run.config.tool_disable = disable.to_vec();
            run.config.tool_force = force.to_vec();
        }
        ToolOverrideScope::Turn => {
            let Some(run) = state.current_run.as_ref() else {
                return Err(ModelError::InvalidValue {
                    field: "current_run",
                    message: "turn-scoped tool overrides require an active run".into(),
                });
            };
            if run.active_turn_id.is_none() {
                return Err(ModelError::InvalidValue {
                    field: "active_turn_id",
                    message: "turn-scoped tool overrides require an active turn".into(),
                });
            }
        }
    }
    Ok(())
}

fn ensure_session_status(state: &SessionState, expected: SessionStatus) -> Result<(), ModelError> {
    if state.status != expected {
        return Err(ModelError::InvalidLifecycleTransition {
            kind: "session_status",
            from: format!("{:?}", state.status),
            to: format!("{:?}", expected),
        });
    }
    Ok(())
}

fn apply_run_lifecycle_changed(
    state: &mut SessionState,
    run_id: &RunId,
    from: RunLifecycle,
    to: RunLifecycle,
    at_ms: u64,
) -> Result<(), ModelError> {
    let Some(current) = state.current_run.as_ref() else {
        return Err(ModelError::InvalidValue {
            field: "current_run",
            message: "no foreground run is active".into(),
        });
    };
    if &current.run_id != run_id {
        return Err(ModelError::InvalidValue {
            field: "run_id",
            message: format!(
                "current run is '{}', event targets '{}'",
                current.run_id, run_id
            ),
        });
    }
    if current.lifecycle != from {
        return Err(ModelError::InvalidLifecycleTransition {
            kind: "run_lifecycle",
            from: format!("{:?}", current.lifecycle),
            to: format!("{:?}", from),
        });
    }

    if to.is_terminal() {
        let outcome = terminal_outcome(to, current);
        state.finish_current_run(to, outcome, at_ms)?;
    } else {
        let current = state.current_run.as_mut().expect("validated current run");
        current.transition_to(to, at_ms)?;
    }
    Ok(())
}

fn terminal_outcome(lifecycle: RunLifecycle, run: &RunState) -> RunOutcome {
    match lifecycle {
        RunLifecycle::Completed => {
            let output_ref = run
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.output_ref.clone())
                .or_else(|| run.latest_output_ref.clone());
            RunOutcome::completed(output_ref)
        }
        RunLifecycle::Failed => run
            .outcome
            .clone()
            .unwrap_or_else(|| RunOutcome::failed("run_failed", "run failed")),
        RunLifecycle::Cancelled => RunOutcome {
            cancelled_reason: Some("cancelled".into()),
            ..Default::default()
        },
        RunLifecycle::Interrupted => RunOutcome::default(),
        RunLifecycle::Queued | RunLifecycle::Running | RunLifecycle::Waiting => {
            RunOutcome::default()
        }
    }
}

fn validate_turn_transition(from: TurnLifecycle, to: TurnLifecycle) -> Result<(), ModelError> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(ModelError::InvalidLifecycleTransition {
            kind: "turn_lifecycle",
            from: format!("{:?}", from),
            to: format!("{:?}", to),
        })
    }
}

fn validate_turn_belongs_to_current_run(
    state: &SessionState,
    turn_id: &TurnId,
) -> Result<(), ModelError> {
    let Some(current) = state.current_run.as_ref() else {
        return Err(ModelError::InvalidValue {
            field: "current_run",
            message: "no foreground run is active".into(),
        });
    };
    if current.run_id != turn_id.run_id {
        return Err(ModelError::InvalidValue {
            field: "turn_id",
            message: "turn does not belong to current run".into(),
        });
    }
    Ok(())
}

fn current_run_for_turn_mut<'a>(
    state: &'a mut SessionState,
    turn_id: &TurnId,
) -> Result<&'a mut RunState, ModelError> {
    validate_turn_belongs_to_current_run(state, turn_id)?;
    Ok(state.current_run.as_mut().expect("validated current run"))
}

fn clear_active_turn(
    state: &mut SessionState,
    turn_id: &TurnId,
    at_ms: u64,
) -> Result<(), ModelError> {
    let run = current_run_for_turn_mut(state, turn_id)?;
    if run.active_turn_id.as_ref() == Some(turn_id) {
        run.active_turn_id = None;
        run.updated_at_ms = at_ms;
        Ok(())
    } else {
        Err(ModelError::InvalidValue {
            field: "active_turn_id",
            message: "turn is not active".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReasoningEffort, RunConfigOverride};
    use crate::effects::{
        AgentEffectIntent, AgentEffectKind, AgentReceiptKind, EffectFailure, EffectStreamFrame,
        EffectStreamFrameKind, LlmGenerationReceipt, LlmGenerationRequest, ToolInvocationReceipt,
        ToolInvocationRequest,
    };
    use crate::events::{AgentEventJoins, HistoryRewriteRequest};
    use crate::ids::{IdAllocator, SessionId, SubmissionId, ToolCallId};
    use crate::refs::BlobRef;
    use crate::tooling::{ToolCallObserved, ToolProfile, ToolRegistry, ToolSpec};
    use serde_json::json;

    fn state() -> SessionState {
        SessionState::new(SessionId::new("session-a"), SessionConfig::default(), 1)
    }

    fn event(seq: u64, kind: AgentEventKind) -> AgentEvent {
        AgentEvent::new("event", SessionId::new("session-a"), seq + 10, kind)
            .with_journal_seq(JournalSeq(seq))
    }

    fn event_with_id(seq: u64, event_id: &str, kind: AgentEventKind) -> AgentEvent {
        AgentEvent::new(event_id, SessionId::new("session-a"), seq + 10, kind)
            .with_journal_seq(JournalSeq(seq))
    }

    fn open_and_queue_run(state: &mut SessionState) {
        apply_events(
            state,
            [
                event(
                    1,
                    AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
                ),
                event(
                    2,
                    AgentEventKind::Input(InputEvent::RunRequested {
                        input_ref: BlobRef::new_unchecked_for_tests("blob://prompt"),
                        run_overrides: None,
                    }),
                ),
            ]
            .iter(),
        )
        .expect("open and queue run");
    }

    fn tool_intent(state: &mut SessionState) -> AgentEffectIntent {
        tool_intent_for(state, ToolCallId::new("call-1"))
    }

    fn tool_intent_for(state: &mut SessionState, call_id: ToolCallId) -> AgentEffectIntent {
        let effect_id = state.id_allocator.allocate_effect_id();
        let run_id = state.current_run.as_ref().expect("run").run_id.clone();
        let mut intent = AgentEffectIntent::new(
            effect_id,
            state.session_id.clone(),
            AgentEffectKind::ToolInvoke(ToolInvocationRequest {
                call_id,
                provider_call_id: None,
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                arguments_json: None,
                arguments_ref: None,
                handler_id: Some("test.echo".into()),
                context_ref: None,
                metadata: Default::default(),
            }),
            30,
        );
        intent.run_id = Some(run_id);
        intent
    }

    fn llm_intent(state: &mut SessionState) -> AgentEffectIntent {
        let effect_id = state.id_allocator.allocate_effect_id();
        let run_id = state.current_run.as_ref().expect("run").run_id.clone();
        let mut intent = AgentEffectIntent::new(
            effect_id,
            state.session_id.clone(),
            AgentEffectKind::LlmComplete(LlmGenerationRequest {
                resolved_context: Default::default(),
                request_ref: None,
                stream: false,
            }),
            30,
        );
        intent.run_id = Some(run_id);
        intent
    }

    fn install_echo_tool(state: &mut SessionState, next_seq: u64) {
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
        apply_event(
            state,
            &event(
                next_seq,
                AgentEventKind::Input(InputEvent::ToolRegistrySet { registry }),
            ),
        )
        .expect("set registry");
        apply_event(
            state,
            &event(
                next_seq + 1,
                AgentEventKind::Input(InputEvent::ToolProfileSelected {
                    profile_id: "local".into(),
                }),
            ),
        )
        .expect("select profile");
    }

    #[test]
    fn input_events_open_session_and_queue_foreground_run_deterministically() {
        let mut state = state();
        let open = event(
            1,
            AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
        );
        apply_event(&mut state, &open).expect("open session");

        let run = event_with_id(
            2,
            "run",
            AgentEventKind::Input(InputEvent::RunRequested {
                input_ref: BlobRef::new_unchecked_for_tests("blob://prompt"),
                run_overrides: Some(RunConfigOverride {
                    model: Some("override-model".into()),
                    ..Default::default()
                }),
            }),
        )
        .with_joins(AgentEventJoins {
            submission_id: Some(SubmissionId::new("submit-1")),
            ..Default::default()
        });
        apply_event(&mut state, &run).expect("request run");

        let current = state.current_run.as_ref().expect("queued foreground run");
        assert_eq!(current.lifecycle, RunLifecycle::Queued);
        assert_eq!(current.run_id.run_seq, 1);
        assert_eq!(current.config_revision, 0);
        assert_eq!(current.config.model, "override-model");
        assert_eq!(
            current.cause.origin,
            crate::state::RunCauseOrigin::DirectSubmission {
                submission_id: Some(SubmissionId::new("submit-1")),
                source: "RunRequested".into(),
                request_ref: None,
            }
        );
        assert_eq!(state.latest_journal_seq, Some(JournalSeq(2)));
    }

    #[test]
    fn follow_up_and_steering_inputs_are_queued() {
        let mut state = state();
        apply_events(
            &mut state,
            [
                event(
                    1,
                    AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
                ),
                event(
                    2,
                    AgentEventKind::Input(InputEvent::RunRequested {
                        input_ref: BlobRef::new_unchecked_for_tests("blob://prompt"),
                        run_overrides: None,
                    }),
                ),
                event(
                    3,
                    AgentEventKind::Input(InputEvent::FollowUpInputAppended {
                        input_ref: BlobRef::new_unchecked_for_tests("blob://follow-up"),
                        run_overrides: None,
                    }),
                ),
                event(
                    4,
                    AgentEventKind::Input(InputEvent::RunSteerRequested {
                        instruction_ref: BlobRef::new_unchecked_for_tests("blob://steer"),
                    }),
                ),
            ]
            .iter(),
        )
        .expect("apply events");

        assert_eq!(state.pending_follow_up_inputs.len(), 1);
        assert_eq!(state.pending_steering_inputs.len(), 1);
        assert_eq!(state.pending_follow_up_inputs[0].queued_at_ms, 13);
        assert_eq!(state.pending_steering_inputs[0].queued_at_ms, 14);
    }

    #[test]
    fn config_and_tool_boundary_inputs_update_revision() {
        let mut state = state();
        apply_event(
            &mut state,
            &event(
                1,
                AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
            ),
        )
        .expect("open");

        let updated = SessionConfig {
            provider: "openai".into(),
            model: "gpt-test".into(),
            reasoning_effort: Some(ReasoningEffort::High),
            ..Default::default()
        };
        apply_event(
            &mut state,
            &event(
                2,
                AgentEventKind::Input(InputEvent::SessionConfigUpdated { config: updated }),
            ),
        )
        .expect("config update");

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
        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Input(InputEvent::ToolRegistrySet { registry }),
            ),
        )
        .expect("registry set");
        apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Input(InputEvent::ToolProfileSelected {
                    profile_id: "local".into(),
                }),
            ),
        )
        .expect("profile selected");

        assert_eq!(state.config_revision, 3);
        assert_eq!(state.config.model, "gpt-test");
        assert_eq!(state.selected_tool_profile.as_deref(), Some("local"));
        assert_eq!(state.config.default_tool_profile.as_deref(), Some("local"));
    }

    #[test]
    fn history_rewrite_and_rollback_requests_update_compact_history_state() {
        let mut state = state();
        let rewrite = event(
            1,
            AgentEventKind::Input(InputEvent::SessionHistoryRewriteRequested {
                request: HistoryRewriteRequest {
                    rewrite_id: "rewrite-1".into(),
                    replacement_blob_refs: vec![BlobRef::new_unchecked_for_tests("blob://rewrite")],
                    ..Default::default()
                },
            }),
        );
        apply_event(&mut state, &rewrite).expect("rewrite");
        let rollback = event(
            2,
            AgentEventKind::Input(InputEvent::SessionHistoryRollbackRequested {
                request: crate::events::HistoryRollbackRequest {
                    rollback_id: "rollback-1".into(),
                    reason_ref: Some(BlobRef::new_unchecked_for_tests("blob://rollback")),
                    ..Default::default()
                },
            }),
        );
        apply_event(&mut state, &rollback).expect("rollback");

        assert_eq!(
            state.history.latest_rewrite_id.as_deref(),
            Some("rewrite-1")
        );
        assert_eq!(
            state.history.latest_rollback_id.as_deref(),
            Some("rollback-1")
        );
        assert_eq!(
            state.history.latest_history_ref,
            Some(BlobRef::new_unchecked_for_tests("blob://rollback"))
        );
        assert_eq!(state.history.rewrite_count, 1);
        assert_eq!(state.history.rollback_count, 1);
    }

    #[test]
    fn lifecycle_events_transition_current_run_and_complete_history() {
        let mut state = state();
        apply_events(
            &mut state,
            [
                event(
                    1,
                    AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
                ),
                event(
                    2,
                    AgentEventKind::Input(InputEvent::RunRequested {
                        input_ref: BlobRef::new_unchecked_for_tests("blob://prompt"),
                        run_overrides: None,
                    }),
                ),
            ]
            .iter(),
        )
        .expect("setup run");
        let run_id = state.current_run.as_ref().expect("run").run_id.clone();

        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Lifecycle(LifecycleEvent::RunLifecycleChanged {
                    run_id: run_id.clone(),
                    from: RunLifecycle::Queued,
                    to: RunLifecycle::Running,
                }),
            ),
        )
        .expect("start run");
        assert_eq!(
            state.current_run.as_ref().expect("current").lifecycle,
            RunLifecycle::Running
        );

        apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Lifecycle(LifecycleEvent::RunLifecycleChanged {
                    run_id: run_id.clone(),
                    from: RunLifecycle::Running,
                    to: RunLifecycle::Completed,
                }),
            ),
        )
        .expect("complete run");

        assert!(state.current_run.is_none());
        assert_eq!(state.run_history.len(), 1);
        assert_eq!(state.run_history[0].run_id, run_id);
        assert_eq!(state.run_history[0].config_revision, 0);
    }

    #[test]
    fn reducer_rejects_out_of_order_and_invalid_lifecycle_events() {
        let mut state = state();
        let missing_seq = AgentEvent::new(
            "event",
            SessionId::new("session-a"),
            1,
            AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
        );
        assert!(matches!(
            apply_event(&mut state, &missing_seq),
            Err(ModelError::InvalidValue {
                field: "journal_seq",
                ..
            })
        ));

        apply_event(
            &mut state,
            &event(
                1,
                AgentEventKind::Input(InputEvent::SessionOpened { config: None }),
            ),
        )
        .expect("open");

        let duplicate_seq = event(1, AgentEventKind::Input(InputEvent::SessionPaused));
        assert!(matches!(
            apply_event(&mut state, &duplicate_seq),
            Err(ModelError::InvalidValue {
                field: "journal_seq",
                ..
            })
        ));

        let invalid = event(
            2,
            AgentEventKind::Lifecycle(LifecycleEvent::SessionStatusChanged {
                from: SessionStatus::Paused,
                to: SessionStatus::Active,
            }),
        );
        assert!(matches!(
            apply_event(&mut state, &invalid),
            Err(ModelError::InvalidLifecycleTransition {
                kind: "session_status",
                ..
            })
        ));
    }

    #[test]
    fn turn_lifecycle_events_track_active_turn() {
        let mut state = state();
        open_and_queue_run(&mut state);
        let run_id = state.current_run.as_ref().expect("run").run_id.clone();
        let turn_id = IdAllocator::new(SessionId::new("session-a")).allocate_turn_id(&run_id);

        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Lifecycle(LifecycleEvent::TurnStarted {
                    turn_id: turn_id.clone(),
                }),
            ),
        )
        .expect("turn start");
        assert_eq!(
            state.current_run.as_ref().expect("run").active_turn_id,
            Some(turn_id.clone())
        );

        apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Lifecycle(LifecycleEvent::TurnCompleted {
                    turn_id: turn_id.clone(),
                }),
            ),
        )
        .expect("turn complete");
        assert!(
            state
                .current_run
                .as_ref()
                .expect("run")
                .active_turn_id
                .is_none()
        );
    }

    #[test]
    fn effect_events_record_stream_and_settle_pending_effects() {
        let mut state = state();
        open_and_queue_run(&mut state);
        let intent = tool_intent(&mut state);
        let effect_id = intent.effect_id.clone();
        let run_id = intent.run_id.clone();

        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: intent.clone(),
                }),
            ),
        )
        .expect("record intent");
        assert!(state.pending_effects.contains_key(&effect_id));
        assert!(
            state
                .current_run
                .as_ref()
                .expect("run")
                .pending_effects
                .contains_key(&effect_id)
        );

        let frame = EffectStreamFrame {
            effect_id: effect_id.clone(),
            session_id: state.session_id.clone(),
            run_id: run_id.clone(),
            turn_id: None,
            sequence: 1,
            observed_at_ms: 14,
            kind: EffectStreamFrameKind::Progress {
                message: "running".into(),
                progress: None,
                total: None,
            },
        };
        apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Effect(EffectEvent::EffectStreamFrameObserved { frame }),
            ),
        )
        .expect("stream frame");
        assert_eq!(
            state.pending_effects[&effect_id].status,
            PendingEffectStatus::Streaming
        );

        let receipt = intent.receipt(
            AgentReceiptKind::ToolInvoke(ToolInvocationReceipt {
                call_id: ToolCallId::new("call-1"),
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                output_ref: Some(BlobRef::new_unchecked_for_tests("blob://tool-output")),
                model_visible_output_ref: Some(BlobRef::new_unchecked_for_tests(
                    "blob://tool-visible",
                )),
                is_error: false,
                metadata: Default::default(),
            }),
            15,
        );
        apply_event(
            &mut state,
            &event(
                5,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded {
                    receipt: receipt.clone(),
                }),
            ),
        )
        .expect("settle receipt");
        assert!(!state.pending_effects.contains_key(&effect_id));
        assert!(
            !state
                .current_run
                .as_ref()
                .expect("run")
                .pending_effects
                .contains_key(&effect_id)
        );

        apply_event(
            &mut state,
            &event(
                6,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }),
            ),
        )
        .expect("duplicate receipt is idempotent");
    }

    #[test]
    fn duplicate_effect_intent_is_rejected_without_mutating_pending_state() {
        let mut state = state();
        open_and_queue_run(&mut state);
        let intent = tool_intent(&mut state);
        let effect_id = intent.effect_id.clone();

        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: intent.clone(),
                }),
            ),
        )
        .expect("record intent");
        let pending_count = state.pending_effects.len();

        let error = apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded { intent }),
            ),
        )
        .expect_err("duplicate intent fails");
        assert!(matches!(
            error,
            ModelError::InvalidValue {
                field: "effect_id",
                ..
            }
        ));
        assert_eq!(state.pending_effects.len(), pending_count);
        assert!(state.pending_effects.contains_key(&effect_id));
        assert_eq!(state.latest_journal_seq, Some(JournalSeq(3)));
    }

    #[test]
    fn non_retryable_effect_failure_is_modelled_on_current_run() {
        let mut state = state();
        open_and_queue_run(&mut state);
        let intent = tool_intent(&mut state);
        let effect_id = intent.effect_id.clone();

        apply_event(
            &mut state,
            &event(
                3,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: intent.clone(),
                }),
            ),
        )
        .expect("record intent");

        let receipt = intent.receipt(
            AgentReceiptKind::Failed(EffectFailure {
                code: "provider_auth".into(),
                detail: "bad credentials".into(),
                retryable: false,
                failure_ref: Some(BlobRef::new_unchecked_for_tests("blob://failure")),
            }),
            14,
        );
        apply_event(
            &mut state,
            &event(
                4,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }),
            ),
        )
        .expect("record failed receipt");

        assert!(!state.pending_effects.contains_key(&effect_id));
        let failure = state
            .current_run
            .as_ref()
            .and_then(|run| run.outcome.as_ref())
            .and_then(|outcome| outcome.failure.as_ref())
            .expect("run failure");
        assert_eq!(failure.code, "provider_auth");
    }

    #[test]
    fn llm_tool_calls_create_active_batch_with_unavailable_failures() {
        let mut state = state();
        open_and_queue_run(&mut state);
        install_echo_tool(&mut state, 3);
        let intent = llm_intent(&mut state);
        let effect_id = intent.effect_id.clone();

        apply_event(
            &mut state,
            &event(
                5,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: intent.clone(),
                }),
            ),
        )
        .expect("record llm intent");
        let receipt = intent.receipt(
            AgentReceiptKind::LlmComplete(LlmGenerationReceipt {
                assistant_message_ref: Some(BlobRef::new_unchecked_for_tests("blob://assistant")),
                tool_calls: vec![
                    ToolCallObserved {
                        call_id: ToolCallId::new("call-echo"),
                        tool_name: "echo".into(),
                        arguments_json: Some(r#"{"text":"hi"}"#.into()),
                        ..Default::default()
                    },
                    ToolCallObserved {
                        call_id: ToolCallId::new("call-missing"),
                        tool_name: "missing".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            16,
        );
        apply_event(
            &mut state,
            &event(
                6,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }),
            ),
        )
        .expect("record llm receipt");

        let run = state.current_run.as_ref().expect("run");
        assert_eq!(run.active_llm_effect_id, None);
        assert!(!run.pending_effects.contains_key(&effect_id));
        assert_eq!(run.latest_output_ref, None);
        let batch = run.active_tool_batch.as_ref().expect("active batch");
        assert_eq!(batch.plan.observed_calls.len(), 2);
        assert_eq!(
            batch.call_status.get(&ToolCallId::new("call-echo")),
            Some(&ToolCallStatus::Queued)
        );
        assert!(matches!(
            batch.call_status.get(&ToolCallId::new("call-missing")),
            Some(ToolCallStatus::Failed { code, .. }) if code == "tool_unavailable"
        ));
        assert!(
            batch
                .model_results
                .get(&ToolCallId::new("call-missing"))
                .expect("unavailable result")
                .is_error
        );
        assert_eq!(batch.execution_groups().len(), 1);
    }

    #[test]
    fn tool_receipts_update_call_status_and_complete_batch() {
        let mut state = state();
        open_and_queue_run(&mut state);
        install_echo_tool(&mut state, 3);
        let llm = llm_intent(&mut state);
        apply_event(
            &mut state,
            &event(
                5,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: llm.clone(),
                }),
            ),
        )
        .expect("record llm intent");
        let llm_receipt = llm.receipt(
            AgentReceiptKind::LlmComplete(LlmGenerationReceipt {
                tool_calls: vec![ToolCallObserved {
                    call_id: ToolCallId::new("call-echo"),
                    tool_name: "echo".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            16,
        );
        apply_event(
            &mut state,
            &event(
                6,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded {
                    receipt: llm_receipt,
                }),
            ),
        )
        .expect("create batch");
        assert!(
            state
                .current_run
                .as_ref()
                .expect("run")
                .active_tool_batch
                .is_some()
        );

        let tool = tool_intent_for(&mut state, ToolCallId::new("call-echo"));
        let tool_effect_id = tool.effect_id.clone();
        apply_event(
            &mut state,
            &event(
                7,
                AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                    intent: tool.clone(),
                }),
            ),
        )
        .expect("record tool intent");
        let batch = state
            .current_run
            .as_ref()
            .expect("run")
            .active_tool_batch
            .as_ref()
            .expect("active batch");
        assert_eq!(
            batch.call_status.get(&ToolCallId::new("call-echo")),
            Some(&ToolCallStatus::Pending)
        );
        assert_eq!(
            batch
                .pending_effects
                .get(&ToolCallId::new("call-echo"))
                .expect("pending tool effect")
                .effect_id,
            tool_effect_id
        );

        let tool_receipt = tool.receipt(
            AgentReceiptKind::ToolInvoke(ToolInvocationReceipt {
                call_id: ToolCallId::new("call-echo"),
                tool_id: Some("echo".into()),
                tool_name: "echo".into(),
                output_ref: Some(BlobRef::new_unchecked_for_tests("blob://tool-output")),
                model_visible_output_ref: Some(BlobRef::new_unchecked_for_tests(
                    "blob://tool-visible",
                )),
                is_error: false,
                metadata: Default::default(),
            }),
            17,
        );
        apply_event(
            &mut state,
            &event(
                8,
                AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded {
                    receipt: tool_receipt,
                }),
            ),
        )
        .expect("settle tool");

        let run = state.current_run.as_ref().expect("run");
        assert!(run.active_tool_batch.is_none());
        assert_eq!(run.completed_tool_batches.len(), 1);
        let batch = &run.completed_tool_batches[0];
        assert_eq!(
            batch.call_status.get(&ToolCallId::new("call-echo")),
            Some(&ToolCallStatus::Succeeded)
        );
        assert_eq!(
            batch
                .model_results
                .get(&ToolCallId::new("call-echo"))
                .expect("tool result")
                .output_ref,
            BlobRef::new_unchecked_for_tests("blob://tool-output")
        );
    }
}
