//! Turn planning extension API and first default planner.

use crate::config::TurnConfig;
use crate::context::{
    ActiveWindowItem, ActiveWindowItemKind, ContextInputLane, ContextOperationPhase,
    ContextOperationState,
};
use crate::ids::{CorrelationId, TurnId};
use crate::refs::BlobRef;
use crate::state::{RunState, SessionState};
use crate::tooling::ToolSpec;
use crate::turn::{
    ResolvedTurnContext, TurnBudget, TurnInput, TurnInputKind, TurnInputLane, TurnPlan,
    TurnPrerequisite, TurnPrerequisiteKind, TurnPriority, TurnReport, TurnTokenEstimate,
    TurnToolChoice,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnPlanningRequest<'a> {
    pub session: &'a SessionState,
    pub run: &'a RunState,
    pub turn_id: TurnId,
    pub inputs: Vec<TurnInput>,
    pub tool_candidates: Vec<ToolCandidate>,
    pub turn_config: Option<TurnConfig>,
    pub current_date: Option<String>,
    pub timezone: Option<String>,
    pub correlation_id: Option<CorrelationId>,
}

impl<'a> TurnPlanningRequest<'a> {
    pub fn from_state(session: &'a SessionState, run: &'a RunState, turn_id: TurnId) -> Self {
        Self {
            inputs: default_inputs(session, run),
            tool_candidates: default_tool_candidates(session, run),
            session,
            run,
            turn_id,
            turn_config: None,
            current_date: None,
            timezone: None,
            correlation_id: None,
        }
    }

    pub fn with_input(mut self, input: TurnInput) -> Self {
        self.inputs.push(input);
        self
    }

    pub fn with_tool_candidate(mut self, candidate: ToolCandidate) -> Self {
        self.tool_candidates.push(candidate);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnPlanningOutcome {
    pub plan: TurnPlan,
    pub resolved_context: ResolvedTurnContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCandidate {
    pub spec: ToolSpec,
    pub source: ToolCandidateSource,
    pub priority: TurnPriority,
    pub enabled: bool,
    pub reason: Option<String>,
    pub estimated_tokens: Option<u64>,
    pub tags: Vec<String>,
}

impl ToolCandidate {
    pub fn enabled(spec: ToolSpec, source: ToolCandidateSource) -> Self {
        let estimated_tokens = spec.estimated_tokens;
        Self {
            spec,
            source,
            priority: TurnPriority::Normal,
            enabled: true,
            reason: None,
            estimated_tokens,
            tags: Vec::new(),
        }
    }

    pub fn disabled(
        spec: ToolSpec,
        source: ToolCandidateSource,
        reason: impl Into<String>,
    ) -> Self {
        let estimated_tokens = spec.estimated_tokens;
        Self {
            spec,
            source,
            priority: TurnPriority::Low,
            enabled: false,
            reason: Some(reason.into()),
            estimated_tokens,
            tags: Vec::new(),
        }
    }

    pub fn required(mut self) -> Self {
        self.priority = TurnPriority::Required;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolCandidateSource {
    #[default]
    RegistryDefault,
    RegistryProfile,
    ConfigEnable,
    ConfigForce,
    Dynamic,
    Runtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlannerError {
    EmptySelection,
}

pub trait TurnPlanner {
    fn plan_turn(
        &self,
        request: TurnPlanningRequest<'_>,
    ) -> Result<TurnPlanningOutcome, PlannerError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTurnPlanner;

impl TurnPlanner for DefaultTurnPlanner {
    fn plan_turn(
        &self,
        request: TurnPlanningRequest<'_>,
    ) -> Result<TurnPlanningOutcome, PlannerError> {
        plan_default_turn(request)
    }
}

pub fn plan_default_turn(
    request: TurnPlanningRequest<'_>,
) -> Result<TurnPlanningOutcome, PlannerError> {
    let budget = request
        .turn_config
        .as_ref()
        .and_then(|config| config.context_budget.as_ref())
        .map(TurnBudget::from)
        .unwrap_or_else(|| TurnBudget::from(&request.run.config.context_budget));

    let mut inputs = request.inputs.clone();
    inputs.sort_by_key(|input| {
        (
            input_kind_rank(input.kind),
            lane_rank(input.lane),
            priority_rank(input.priority),
            input.input_id.clone(),
        )
    });

    let mut active_window_items = Vec::new();
    let mut response_format_ref = None;
    let mut provider_options_ref = None;
    let mut seen_refs = BTreeSet::new();
    let mut selected_message_count = 0_u64;
    let mut dropped_message_count = 0_u64;
    let mut message_tokens = 0_u64;
    let mut unknown_message_count = 0_u64;
    let mut decision_codes = Vec::new();
    let mut unresolved = Vec::new();

    for input in inputs {
        match input.kind {
            TurnInputKind::MessageRef => {
                if !seen_refs.insert(input.content_ref.as_str().to_string()) {
                    dropped_message_count = dropped_message_count.saturating_add(1);
                    decision_codes.push(format!("drop_message_duplicate:{}", input.input_id));
                    continue;
                }
                let required = matches!(input.priority, TurnPriority::Required);
                if over_ref_budget(active_window_items.len(), budget.max_message_refs) && !required
                {
                    dropped_message_count = dropped_message_count.saturating_add(1);
                    decision_codes.push(format!("drop_message_ref_budget:{}", input.input_id));
                    continue;
                }
                if over_token_budget(
                    message_tokens,
                    input.estimated_tokens,
                    budget.max_input_tokens,
                ) && !required
                {
                    dropped_message_count = dropped_message_count.saturating_add(1);
                    decision_codes.push(format!("drop_message_token_budget:{}", input.input_id));
                    continue;
                }
                if let Some(tokens) = input.estimated_tokens {
                    message_tokens = message_tokens.saturating_add(tokens);
                } else {
                    unknown_message_count = unknown_message_count.saturating_add(1);
                }
                selected_message_count = selected_message_count.saturating_add(1);
                active_window_items.push(ActiveWindowItem::message_ref(
                    input.input_id,
                    input.content_ref,
                    Some(context_lane(input.lane)),
                    None,
                ));
            }
            TurnInputKind::ResponseFormatRef => {
                if response_format_ref.is_none() {
                    response_format_ref = Some(input.content_ref);
                    decision_codes.push("select_response_format".into());
                } else {
                    dropped_message_count = dropped_message_count.saturating_add(1);
                    decision_codes.push(format!("drop_response_format_extra:{}", input.input_id));
                }
            }
            TurnInputKind::ProviderOptionsRef => {
                if provider_options_ref.is_none() {
                    provider_options_ref = Some(input.content_ref);
                    decision_codes.push("select_provider_options".into());
                } else {
                    dropped_message_count = dropped_message_count.saturating_add(1);
                    decision_codes.push(format!("drop_provider_options_extra:{}", input.input_id));
                }
            }
            TurnInputKind::BlobRef | TurnInputKind::ToolDefinitionRef | TurnInputKind::Custom => {
                dropped_message_count = dropped_message_count.saturating_add(1);
                decision_codes.push(format!("drop_non_message_input:{}", input.input_id));
            }
        }
    }

    if active_window_items.is_empty() {
        return Err(PlannerError::EmptySelection);
    }

    let (
        selected_tool_ids,
        model_visible_tools,
        dropped_tool_count,
        tool_tokens,
        unknown_tool_count,
    ) = select_tools(
        &request.tool_candidates,
        message_tokens,
        budget.clone(),
        &mut decision_codes,
    );

    let mut prerequisites = Vec::new();
    if let Some(prerequisite) = context_operation_prerequisite(
        request
            .session
            .context_state
            .pending_context_operation
            .as_ref(),
    ) {
        unresolved.push("context_operation_pending".into());
        prerequisites.push(prerequisite);
    }
    if !prerequisites.is_empty() {
        unresolved.push("prerequisites_pending".into());
    }

    let selected_tool_count = selected_tool_ids.len() as u64;
    let token_estimate = TurnTokenEstimate {
        message_tokens,
        tool_tokens,
        total_input_tokens: message_tokens.saturating_add(tool_tokens),
        unknown_message_count,
        unknown_tool_count,
    };
    decision_codes.push(format!(
        "selected_turn:session={}:run={}",
        request.session.session_id, request.run.run_id.run_seq
    ));

    let plan = TurnPlan {
        turn_id: Some(request.turn_id.clone()),
        active_window_items,
        selected_tool_ids,
        tool_choice: if model_visible_tools.is_empty() {
            None
        } else {
            Some(TurnToolChoice::Auto)
        },
        response_format_ref,
        provider_options_ref,
        prerequisites,
        report: TurnReport {
            planner: "forge.agent/default-turn".into(),
            selected_message_count,
            dropped_message_count,
            selected_tool_count,
            dropped_tool_count,
            token_estimate,
            budget,
            decision_codes,
            unresolved,
        },
        ..Default::default()
    };

    let mut resolved_context = ResolvedTurnContext::from_run_and_plan(
        request.session.session_id.clone(),
        request.run.run_id.clone(),
        request.turn_id,
        request.run.effective_agent_version_id.clone(),
        request.run.config_revision,
        &request.run.config,
        request.turn_config.as_ref(),
        &plan,
        model_visible_tools,
    );
    resolved_context.current_date = request.current_date;
    resolved_context.timezone = request.timezone;
    resolved_context.correlation_id = request.correlation_id;

    Ok(TurnPlanningOutcome {
        plan,
        resolved_context,
    })
}

fn select_tools(
    candidates: &[ToolCandidate],
    message_tokens: u64,
    budget: TurnBudget,
    decision_codes: &mut Vec<String>,
) -> (Vec<String>, Vec<ToolSpec>, u64, u64, u64) {
    let mut selected_ids = Vec::new();
    let mut selected_specs = Vec::new();
    let mut dropped = 0_u64;
    let mut tool_tokens = 0_u64;
    let mut unknown_tools = 0_u64;
    let mut seen = BTreeSet::new();

    for candidate in candidates {
        if !seen.insert(candidate.spec.tool_id.clone()) {
            dropped = dropped.saturating_add(1);
            decision_codes.push(format!("drop_tool_duplicate:{}", candidate.spec.tool_id));
            continue;
        }
        if !candidate.enabled {
            dropped = dropped.saturating_add(1);
            decision_codes.push(format!(
                "drop_tool_disabled:{}:{}",
                candidate.spec.tool_id,
                candidate.reason.clone().unwrap_or_default()
            ));
            continue;
        }
        let required = matches!(candidate.priority, TurnPriority::Required);
        if over_ref_budget(selected_ids.len(), budget.max_tool_refs) && !required {
            dropped = dropped.saturating_add(1);
            decision_codes.push(format!("drop_tool_ref_budget:{}", candidate.spec.tool_id));
            continue;
        }
        if over_token_budget(
            message_tokens.saturating_add(tool_tokens),
            candidate.estimated_tokens,
            budget.max_input_tokens,
        ) && !required
        {
            dropped = dropped.saturating_add(1);
            decision_codes.push(format!("drop_tool_token_budget:{}", candidate.spec.tool_id));
            continue;
        }
        if let Some(tokens) = candidate.estimated_tokens {
            tool_tokens = tool_tokens.saturating_add(tokens);
        } else {
            unknown_tools = unknown_tools.saturating_add(1);
        }
        selected_ids.push(candidate.spec.tool_id.clone());
        selected_specs.push(candidate.spec.clone());
    }

    (
        selected_ids,
        selected_specs,
        dropped,
        tool_tokens,
        unknown_tools,
    )
}

fn default_inputs(session: &SessionState, run: &RunState) -> Vec<TurnInput> {
    let mut inputs = Vec::new();
    for (index, prompt_ref) in run.config.prompt_refs.iter().enumerate() {
        inputs.push(message_input(
            format!("prompt-{index}"),
            TurnInputLane::System,
            TurnPriority::Required,
            prompt_ref.clone(),
            "run_prompt",
        ));
    }
    for item in &session.context_state.active_window_items {
        inputs.push(input_from_active_window(item));
    }
    for (index, input_ref) in run.input_refs.iter().enumerate() {
        inputs.push(message_input(
            format!("run-input-{index}"),
            TurnInputLane::Conversation,
            TurnPriority::Required,
            input_ref.clone(),
            "run_input",
        ));
    }
    for (index, steering) in session.pending_steering_inputs.iter().enumerate() {
        inputs.push(message_input(
            format!("steer-{index}"),
            TurnInputLane::Steer,
            TurnPriority::Required,
            steering.instruction_ref.clone(),
            "steering",
        ));
    }
    for batch in &run.completed_tool_batches {
        for result in batch.model_results.values() {
            inputs.push(message_input(
                format!("tool-result-{}", result.call_id),
                TurnInputLane::ToolResult,
                TurnPriority::Required,
                result
                    .model_visible_output_ref
                    .clone()
                    .unwrap_or_else(|| result.output_ref.clone()),
                "tool_result",
            ));
        }
    }
    inputs
}

fn default_tool_candidates(session: &SessionState, run: &RunState) -> Vec<ToolCandidate> {
    let forced = run
        .config
        .tool_force
        .iter()
        .chain(session.config.default_tool_force.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let disabled = run
        .config
        .tool_disable
        .iter()
        .chain(session.config.default_tool_disable.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let enabled_filter = run
        .config
        .tool_enable
        .iter()
        .chain(session.config.default_tool_enable.iter())
        .cloned()
        .collect::<BTreeSet<_>>();

    let tools = run
        .config
        .tool_profile
        .as_ref()
        .or(session.selected_tool_profile.as_ref())
        .or(session.config.default_tool_profile.as_ref())
        .map(|profile_id| {
            session
                .tool_registry
                .model_visible_tools_for_profile(profile_id)
        })
        .unwrap_or_else(|| {
            session
                .tool_registry
                .tools_by_id
                .values()
                .cloned()
                .collect()
        });

    tools
        .into_iter()
        .map(|tool| {
            let source =
                if run.config.tool_profile.is_some() || session.selected_tool_profile.is_some() {
                    ToolCandidateSource::RegistryProfile
                } else {
                    ToolCandidateSource::RegistryDefault
                };
            if disabled.contains(&tool.tool_id) {
                return ToolCandidate::disabled(tool, source, "disabled by config");
            }
            if !enabled_filter.is_empty() && !enabled_filter.contains(&tool.tool_id) {
                return ToolCandidate::disabled(tool, source, "not enabled by config");
            }
            let mut candidate = ToolCandidate::enabled(tool, source);
            if forced.contains(&candidate.spec.tool_id) {
                candidate = candidate.required();
                candidate.source = ToolCandidateSource::ConfigForce;
            }
            candidate
        })
        .collect()
}

fn message_input(
    input_id: impl Into<String>,
    lane: TurnInputLane,
    priority: TurnPriority,
    content_ref: BlobRef,
    source_kind: impl Into<String>,
) -> TurnInput {
    TurnInput {
        input_id: input_id.into(),
        lane,
        kind: TurnInputKind::MessageRef,
        priority,
        content_ref,
        source_kind: Some(source_kind.into()),
        ..Default::default()
    }
}

fn input_from_active_window(item: &ActiveWindowItem) -> TurnInput {
    TurnInput {
        input_id: format!("active-window-{}", item.item_id),
        lane: item
            .lane
            .map(turn_lane)
            .unwrap_or(TurnInputLane::Conversation),
        kind: match item.kind {
            ActiveWindowItemKind::ResponseFormatRef => TurnInputKind::ResponseFormatRef,
            ActiveWindowItemKind::ProviderOptionsRef => TurnInputKind::ProviderOptionsRef,
            ActiveWindowItemKind::ToolDefinitionRef => TurnInputKind::ToolDefinitionRef,
            ActiveWindowItemKind::Custom | ActiveWindowItemKind::ProviderNativeBlobRef => {
                TurnInputKind::BlobRef
            }
            ActiveWindowItemKind::MessageRef
            | ActiveWindowItemKind::SummaryRef
            | ActiveWindowItemKind::ProviderRawWindowRef => TurnInputKind::MessageRef,
        },
        priority: TurnPriority::Normal,
        content_ref: item.content_ref.clone(),
        estimated_tokens: item.estimated_tokens,
        source_kind: Some("active_window".into()),
        source_id: Some(item.item_id.clone()),
        tags: Vec::new(),
        correlation_id: None,
    }
}

fn context_operation_prerequisite(
    operation: Option<&ContextOperationState>,
) -> Option<TurnPrerequisite> {
    let operation = operation?;
    if !operation.blocks_generation() {
        return None;
    }
    Some(TurnPrerequisite {
        prerequisite_id: format!("context_operation:{}", operation.operation_id),
        kind: if matches!(operation.phase, ContextOperationPhase::CountingTokens) {
            TurnPrerequisiteKind::CountTokens
        } else {
            TurnPrerequisiteKind::CompactContext
        },
        reason: format!("context operation '{}' is pending", operation.operation_id),
        input_ids: Vec::new(),
        tool_ids: Vec::new(),
    })
}

fn input_kind_rank(kind: TurnInputKind) -> u8 {
    match kind {
        TurnInputKind::ProviderOptionsRef => 0,
        TurnInputKind::ResponseFormatRef => 1,
        TurnInputKind::MessageRef => 2,
        TurnInputKind::ToolDefinitionRef => 3,
        TurnInputKind::BlobRef => 4,
        TurnInputKind::Custom => 5,
    }
}

fn lane_rank(lane: TurnInputLane) -> u8 {
    match lane {
        TurnInputLane::System => 0,
        TurnInputLane::Developer => 1,
        TurnInputLane::Summary => 2,
        TurnInputLane::Skill => 3,
        TurnInputLane::Memory => 4,
        TurnInputLane::Domain => 5,
        TurnInputLane::RuntimeHint => 6,
        TurnInputLane::Conversation => 7,
        TurnInputLane::ToolResult => 8,
        TurnInputLane::Steer => 9,
        TurnInputLane::Plugin => 10,
        TurnInputLane::App => 11,
        TurnInputLane::Custom => 12,
    }
}

fn priority_rank(priority: TurnPriority) -> u8 {
    match priority {
        TurnPriority::Required => 0,
        TurnPriority::High => 1,
        TurnPriority::Normal => 2,
        TurnPriority::Low => 3,
    }
}

fn over_ref_budget(current_len: usize, max: Option<u64>) -> bool {
    max.is_some_and(|max| current_len >= max as usize)
}

fn over_token_budget(current_tokens: u64, candidate: Option<u64>, max: Option<u64>) -> bool {
    match (candidate, max) {
        (Some(candidate), Some(max)) => current_tokens.saturating_add(candidate) > max,
        _ => false,
    }
}

fn context_lane(lane: TurnInputLane) -> ContextInputLane {
    match lane {
        TurnInputLane::System => ContextInputLane::System,
        TurnInputLane::Developer => ContextInputLane::Developer,
        TurnInputLane::ToolResult => ContextInputLane::ToolResult,
        TurnInputLane::Steer => ContextInputLane::Steer,
        TurnInputLane::Summary => ContextInputLane::Summary,
        TurnInputLane::Memory => ContextInputLane::Memory,
        TurnInputLane::Skill => ContextInputLane::Skill,
        TurnInputLane::Domain => ContextInputLane::Domain,
        TurnInputLane::RuntimeHint => ContextInputLane::RuntimeHint,
        TurnInputLane::Conversation
        | TurnInputLane::Plugin
        | TurnInputLane::App
        | TurnInputLane::Custom => ContextInputLane::Conversation,
    }
}

fn turn_lane(lane: ContextInputLane) -> TurnInputLane {
    match lane {
        ContextInputLane::System => TurnInputLane::System,
        ContextInputLane::Developer => TurnInputLane::Developer,
        ContextInputLane::Conversation => TurnInputLane::Conversation,
        ContextInputLane::ToolResult => TurnInputLane::ToolResult,
        ContextInputLane::Steer => TurnInputLane::Steer,
        ContextInputLane::Summary => TurnInputLane::Summary,
        ContextInputLane::Memory => TurnInputLane::Memory,
        ContextInputLane::Skill => TurnInputLane::Skill,
        ContextInputLane::Domain => TurnInputLane::Domain,
        ContextInputLane::RuntimeHint => TurnInputLane::RuntimeHint,
        ContextInputLane::Custom => TurnInputLane::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{ActiveToolBatch, ToolCallModelResult, ToolCallStatus};
    use crate::config::{ContextBudgetConfig, RunConfig, SessionConfig};
    use crate::context::{
        CompactionStrategy, ContextOperationPhase, ContextOperationState, ContextPressureReason,
    };
    use crate::ids::{SessionId, ToolCallId};
    use crate::refs::BlobRef;
    use crate::state::RunCause;
    use crate::tooling::{
        PlannedToolCall, ToolBatchPlan, ToolCallObserved, ToolProfile, ToolRegistry,
    };
    use serde_json::json;

    fn base_state_and_run() -> (SessionState, RunState, TurnId) {
        let mut state = SessionState::new(SessionId::new("session-a"), SessionConfig::default(), 1);
        let run_id = state.id_allocator.allocate_run_id();
        let turn_id = state.id_allocator.allocate_turn_id(&run_id);
        let run = RunState::queued(
            run_id,
            RunCause::direct_input(BlobRef::new_unchecked_for_tests("blob://prompt"), None),
            None,
            0,
            RunConfig::from_session(&state.config, None),
            2,
        );
        (state, run, turn_id)
    }

    fn echo_tool() -> ToolSpec {
        ToolSpec::new("echo", "echo", "Echo", json!({"type":"object"}))
    }

    #[test]
    fn default_planner_orders_inputs_and_applies_message_budget() {
        let (state, run, turn_id) = base_state_and_run();
        let request =
            TurnPlanningRequest::from_state(&state, &run, turn_id).with_input(TurnInput {
                input_id: "summary".into(),
                lane: TurnInputLane::Summary,
                kind: TurnInputKind::MessageRef,
                priority: TurnPriority::High,
                content_ref: BlobRef::new_unchecked_for_tests("blob://summary"),
                estimated_tokens: Some(10),
                ..Default::default()
            });
        let mut request = request;
        request.turn_config = Some(TurnConfig {
            context_budget: Some(ContextBudgetConfig {
                max_message_refs: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        });

        let outcome = DefaultTurnPlanner.plan_turn(request).expect("plan");

        assert_eq!(outcome.plan.active_window_items.len(), 2);
        assert_eq!(
            outcome.plan.active_window_items[0].content_ref,
            BlobRef::new_unchecked_for_tests("blob://summary")
        );
        assert_eq!(
            outcome.plan.active_window_items[1].content_ref,
            BlobRef::new_unchecked_for_tests("blob://prompt")
        );
        assert_eq!(outcome.plan.report.selected_message_count, 2);
    }

    #[test]
    fn default_planner_selects_dynamic_tool_candidates() {
        let (state, run, turn_id) = base_state_and_run();
        let request = TurnPlanningRequest::from_state(&state, &run, turn_id).with_tool_candidate(
            ToolCandidate::enabled(echo_tool(), ToolCandidateSource::Dynamic),
        );

        let outcome = DefaultTurnPlanner.plan_turn(request).expect("plan");

        assert_eq!(outcome.plan.selected_tool_ids, vec!["echo"]);
        assert_eq!(outcome.resolved_context.model_visible_tools.len(), 1);
        assert_eq!(
            outcome.resolved_context.model_visible_tools[0].tool_name,
            "echo"
        );
    }

    #[test]
    fn default_planner_respects_disabled_and_forced_tool_candidates() {
        let (mut state, mut run, turn_id) = base_state_and_run();
        let mut registry = ToolRegistry::default();
        let mut echo = echo_tool();
        echo.estimated_tokens = Some(10);
        let mut slow = ToolSpec::new("slow", "slow", "Slow", json!({"type":"object"}));
        slow.estimated_tokens = Some(100);
        registry.insert_tool(echo);
        registry.insert_tool(slow);
        registry.insert_profile(ToolProfile {
            profile_id: "local".into(),
            tool_ids: vec!["echo".into(), "slow".into()],
            ..Default::default()
        });
        state.tool_registry = registry;
        state.selected_tool_profile = Some("local".into());
        run.config.tool_disable = vec!["echo".into()];
        run.config.tool_force = vec!["slow".into()];
        let mut request = TurnPlanningRequest::from_state(&state, &run, turn_id);
        request.turn_config = Some(TurnConfig {
            context_budget: Some(ContextBudgetConfig {
                max_input_tokens: Some(20),
                ..Default::default()
            }),
            ..Default::default()
        });

        let outcome = DefaultTurnPlanner.plan_turn(request).expect("plan");

        assert_eq!(outcome.plan.selected_tool_ids, vec!["slow"]);
        assert!(
            outcome
                .plan
                .report
                .decision_codes
                .iter()
                .any(|code| code.starts_with("drop_tool_disabled:echo"))
        );
    }

    #[test]
    fn default_planner_includes_tool_results_and_context_prerequisites() {
        let (mut state, mut run, turn_id) = base_state_and_run();
        let call_id = ToolCallId::new("call-1");
        let plan = ToolBatchPlan::from_planned_calls(
            vec![ToolCallObserved {
                call_id: call_id.clone(),
                tool_name: "echo".into(),
                ..Default::default()
            }],
            vec![PlannedToolCall {
                call_id: call_id.clone(),
                accepted: true,
                ..Default::default()
            }],
        );
        let mut batch = ActiveToolBatch::new(
            state.id_allocator.allocate_tool_batch_id(&run.run_id),
            state.id_allocator.allocate_effect_id(),
            None,
            plan,
        );
        batch.set_call_status(call_id.clone(), ToolCallStatus::Succeeded);
        batch.model_results.insert(
            call_id.clone(),
            ToolCallModelResult {
                call_id,
                tool_id: Some("echo".into()),
                tool_name: "echo".into(),
                is_error: false,
                output_ref: BlobRef::new_unchecked_for_tests("blob://tool-output"),
                model_visible_output_ref: Some(BlobRef::new_unchecked_for_tests(
                    "blob://tool-visible",
                )),
            },
        );
        run.completed_tool_batches.push(batch);
        state
            .context_state
            .set_pending_operation(ContextOperationState {
                operation_id: "ctx-op-1".into(),
                phase: ContextOperationPhase::Compacting,
                reason: ContextPressureReason::UsageHighWater,
                strategy: CompactionStrategy::Summary,
                ..Default::default()
            });

        let outcome = DefaultTurnPlanner
            .plan_turn(TurnPlanningRequest::from_state(&state, &run, turn_id))
            .expect("plan");

        assert!(outcome.plan.active_window_items.iter().any(
            |item| item.content_ref == BlobRef::new_unchecked_for_tests("blob://tool-visible")
        ));
        assert!(matches!(
            outcome.plan.prerequisites.first().map(|value| &value.kind),
            Some(TurnPrerequisiteKind::CompactContext)
        ));
        assert!(!outcome.plan.is_ready_for_generation());
    }
}
