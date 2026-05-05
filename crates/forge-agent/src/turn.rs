//! Turn planning and resolved turn context records.
//!
//! This module will contain input lanes, priorities, budgets, prerequisites,
//! turn plans, reports, and immutable resolved turn context snapshots.

use crate::config::{ContextBudgetConfig, ReasoningEffort, RunConfig, TurnConfig};
use crate::context::ActiveWindowItem;
use crate::ids::{CorrelationId, RunId, SessionId, SubmissionId, TurnId};
use crate::refs::ArtifactRef;
use crate::tooling::ToolSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnInputLane {
    System,
    Developer,
    #[default]
    Conversation,
    ToolResult,
    Steer,
    Summary,
    Memory,
    Skill,
    Plugin,
    App,
    Domain,
    RuntimeHint,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnInputKind {
    #[default]
    MessageRef,
    ResponseFormatRef,
    ProviderOptionsRef,
    ArtifactRef,
    ToolDefinitionRef,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPriority {
    Required,
    High,
    #[default]
    Normal,
    Low,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnBudget {
    pub max_input_tokens: Option<u64>,
    pub reserve_output_tokens: Option<u64>,
    pub max_message_refs: Option<u64>,
    pub max_tool_refs: Option<u64>,
}

impl From<&ContextBudgetConfig> for TurnBudget {
    fn from(value: &ContextBudgetConfig) -> Self {
        Self {
            max_input_tokens: value.max_input_tokens,
            reserve_output_tokens: value.reserve_output_tokens,
            max_message_refs: value.max_message_refs,
            max_tool_refs: value.max_tool_refs,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnInput {
    pub input_id: String,
    pub lane: TurnInputLane,
    pub kind: TurnInputKind,
    pub priority: TurnPriority,
    pub content_ref: ArtifactRef,
    pub estimated_tokens: Option<u64>,
    pub source_kind: Option<String>,
    pub source_id: Option<String>,
    pub correlation_id: Option<CorrelationId>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnToolInput {
    pub tool_id: String,
    pub priority: TurnPriority,
    pub estimated_tokens: Option<u64>,
    pub source_kind: Option<String>,
    pub source_id: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnToolChoice {
    #[default]
    Auto,
    None,
    Required,
    Tool {
        name: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTokenEstimate {
    pub message_tokens: u64,
    pub tool_tokens: u64,
    pub total_input_tokens: u64,
    pub unknown_message_count: u64,
    pub unknown_tool_count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerStateRef {
    pub planner_id: String,
    pub key: String,
    pub state_ref: ArtifactRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnReport {
    pub planner: String,
    pub selected_message_count: u64,
    pub dropped_message_count: u64,
    pub selected_tool_count: u64,
    pub dropped_tool_count: u64,
    pub token_estimate: TurnTokenEstimate,
    pub budget: TurnBudget,
    pub decision_codes: Vec<String>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPrerequisiteKind {
    MaterializeToolDefinitions,
    PrepareToolRuntime,
    CompactContext,
    CountTokens,
    #[default]
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnPrerequisite {
    pub prerequisite_id: String,
    pub kind: TurnPrerequisiteKind,
    pub reason: String,
    pub input_ids: Vec<String>,
    pub tool_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TurnStateUpdate {
    UpsertPinnedInput { input: TurnInput },
    RemovePinnedInput { input_id: String },
    UpsertDurableInput { input: TurnInput },
    RemoveDurableInput { input_id: String },
    UpsertCustomStateRef { state_ref: PlannerStateRef },
    RemoveCustomStateRef { planner_id: String, key: String },
    Noop,
}

impl Default for TurnStateUpdate {
    fn default() -> Self {
        Self::Noop
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnPlan {
    pub turn_id: Option<TurnId>,
    pub active_window_items: Vec<ActiveWindowItem>,
    pub selected_tool_ids: Vec<String>,
    pub tool_choice: Option<TurnToolChoice>,
    pub response_format_ref: Option<ArtifactRef>,
    pub provider_options_ref: Option<ArtifactRef>,
    pub prerequisites: Vec<TurnPrerequisite>,
    pub state_updates: Vec<TurnStateUpdate>,
    pub report: TurnReport,
}

impl TurnPlan {
    pub fn is_ready_for_generation(&self) -> bool {
        self.prerequisites.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    pub submission_id: Option<SubmissionId>,
    pub correlation_id: Option<CorrelationId>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTurnContext {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_output_tokens: Option<u64>,
    pub current_date: Option<String>,
    pub timezone: Option<String>,
    pub base_context_refs: Vec<ArtifactRef>,
    pub developer_context_refs: Vec<ArtifactRef>,
    pub user_context_refs: Vec<ArtifactRef>,
    pub skill_context_refs: Vec<ArtifactRef>,
    pub plugin_context_refs: Vec<ArtifactRef>,
    pub app_context_refs: Vec<ArtifactRef>,
    pub domain_context_refs: Vec<ArtifactRef>,
    pub runtime_context_refs: Vec<ArtifactRef>,
    pub selected_tool_profile: Option<String>,
    pub model_visible_tools: Vec<ToolSpec>,
    pub active_window_items: Vec<ActiveWindowItem>,
    pub tool_choice: Option<TurnToolChoice>,
    pub response_format_ref: Option<ArtifactRef>,
    pub provider_options_ref: Option<ArtifactRef>,
    pub budget: TurnBudget,
    pub trace: TraceContext,
    pub extension_context_refs: Vec<ArtifactRef>,
}

impl ResolvedTurnContext {
    pub fn from_run_and_plan(
        session_id: SessionId,
        run_id: RunId,
        turn_id: TurnId,
        run_config: &RunConfig,
        turn_config: Option<&TurnConfig>,
        plan: &TurnPlan,
        model_visible_tools: Vec<ToolSpec>,
    ) -> Self {
        let budget = turn_config
            .and_then(|config| config.context_budget.as_ref())
            .map(TurnBudget::from)
            .unwrap_or_else(|| TurnBudget::from(&run_config.context_budget));

        Self {
            session_id,
            run_id,
            turn_id,
            provider: turn_config
                .and_then(|config| config.provider.clone())
                .unwrap_or_else(|| run_config.provider.clone()),
            model: turn_config
                .and_then(|config| config.model.clone())
                .unwrap_or_else(|| run_config.model.clone()),
            reasoning_effort: turn_config
                .and_then(|config| config.reasoning_effort)
                .or(run_config.reasoning_effort),
            max_output_tokens: turn_config
                .and_then(|config| config.max_output_tokens)
                .or(run_config.max_output_tokens),
            selected_tool_profile: run_config.tool_profile.clone(),
            model_visible_tools,
            active_window_items: plan.active_window_items.clone(),
            tool_choice: plan.tool_choice.clone(),
            response_format_ref: turn_config
                .and_then(|config| config.response_format_ref.clone())
                .or_else(|| plan.response_format_ref.clone()),
            provider_options_ref: turn_config
                .and_then(|config| config.provider_options_ref.clone())
                .or_else(|| plan.provider_options_ref.clone()),
            budget,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ContextBudgetConfig;
    use crate::ids::IdAllocator;

    #[test]
    fn turn_plan_reports_readiness_from_prerequisites() {
        let ready = TurnPlan::default();
        assert!(ready.is_ready_for_generation());

        let blocked = TurnPlan {
            prerequisites: vec![TurnPrerequisite {
                prerequisite_id: "tool-runtime".into(),
                kind: TurnPrerequisiteKind::PrepareToolRuntime,
                reason: "selected tools require runner preparation".into(),
                input_ids: Vec::new(),
                tool_ids: vec!["shell".into()],
            }],
            ..Default::default()
        };
        assert!(!blocked.is_ready_for_generation());
    }

    #[test]
    fn resolved_turn_context_applies_turn_overrides_deterministically() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let run_id = ids.allocate_run_id();
        let turn_id = ids.allocate_turn_id(&run_id);
        let run_config = RunConfig {
            provider: "openai".into(),
            model: "base-model".into(),
            reasoning_effort: Some(ReasoningEffort::Medium),
            context_budget: ContextBudgetConfig {
                max_input_tokens: Some(1000),
                ..Default::default()
            },
            ..Default::default()
        };
        let turn_config = TurnConfig {
            model: Some("turn-model".into()),
            reasoning_effort: Some(ReasoningEffort::High),
            context_budget: Some(ContextBudgetConfig {
                max_input_tokens: Some(500),
                ..Default::default()
            }),
            ..Default::default()
        };

        let resolved = ResolvedTurnContext::from_run_and_plan(
            ids.session_id.clone(),
            run_id.clone(),
            turn_id.clone(),
            &run_config,
            Some(&turn_config),
            &TurnPlan::default(),
            Vec::new(),
        );

        assert_eq!(resolved.session_id, ids.session_id);
        assert_eq!(resolved.run_id, run_id);
        assert_eq!(resolved.turn_id, turn_id);
        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.model, "turn-model");
        assert_eq!(resolved.reasoning_effort, Some(ReasoningEffort::High));
        assert_eq!(resolved.budget.max_input_tokens, Some(500));
    }
}
