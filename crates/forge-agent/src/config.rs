//! Session, run, and turn configuration records.
//!
//! This module will contain first-cut core configuration only. Hook, approval,
//! permission, sandbox, and policy configuration is deferred.

use crate::ids::AgentVersionId;
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudgetConfig {
    pub max_input_tokens: Option<u64>,
    pub reserve_output_tokens: Option<u64>,
    pub usage_high_water_tokens: Option<u64>,
    pub max_message_refs: Option<u64>,
    pub max_tool_refs: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopLimitConfig {
    pub max_turns: Option<u64>,
    pub max_tool_batches_per_run: Option<u64>,
    pub max_subagent_depth: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLimitConfig {
    pub default_output_bytes: Option<u64>,
    pub per_tool_output_bytes: BTreeMap<String, u64>,
    pub default_output_lines: Option<u64>,
    pub per_tool_output_lines: BTreeMap<String, u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CxdbPersistenceMode {
    Off,
    #[default]
    Required,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub initial_agent_version_id: Option<AgentVersionId>,
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_output_tokens: Option<u64>,
    pub default_prompt_refs: Vec<ArtifactRef>,
    pub default_tool_profile: Option<String>,
    pub default_tool_enable: Vec<String>,
    pub default_tool_disable: Vec<String>,
    pub default_tool_force: Vec<String>,
    pub context_budget: ContextBudgetConfig,
    pub loop_limits: LoopLimitConfig,
    pub tool_limits: ToolLimitConfig,
    pub thread_key: Option<String>,
    pub cxdb_persistence: CxdbPersistenceMode,
    pub extension_config_refs: Vec<ArtifactRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    pub provider: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_output_tokens: Option<u64>,
    pub prompt_refs: Vec<ArtifactRef>,
    pub tool_profile: Option<String>,
    pub tool_enable: Vec<String>,
    pub tool_disable: Vec<String>,
    pub tool_force: Vec<String>,
    pub context_budget: ContextBudgetConfig,
    pub loop_limits: LoopLimitConfig,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_output_tokens: Option<u64>,
    pub context_budget: Option<ContextBudgetConfig>,
    pub response_format_ref: Option<ArtifactRef>,
    pub provider_options_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfigOverride {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub max_output_tokens: Option<u64>,
    pub prompt_refs: Option<Vec<ArtifactRef>>,
    pub tool_profile: Option<String>,
    pub context_budget: Option<ContextBudgetConfig>,
    pub loop_limits: Option<LoopLimitConfig>,
}

impl RunConfig {
    pub fn from_session(session: &SessionConfig, override_: Option<&RunConfigOverride>) -> Self {
        let mut run = Self {
            provider: session.provider.clone(),
            model: session.model.clone(),
            reasoning_effort: session.reasoning_effort,
            max_output_tokens: session.max_output_tokens,
            prompt_refs: session.default_prompt_refs.clone(),
            tool_profile: session.default_tool_profile.clone(),
            tool_enable: session.default_tool_enable.clone(),
            tool_disable: session.default_tool_disable.clone(),
            tool_force: session.default_tool_force.clone(),
            context_budget: session.context_budget.clone(),
            loop_limits: session.loop_limits.clone(),
        };

        if let Some(override_) = override_ {
            if let Some(provider) = &override_.provider {
                run.provider = provider.clone();
            }
            if let Some(model) = &override_.model {
                run.model = model.clone();
            }
            if override_.reasoning_effort.is_some() {
                run.reasoning_effort = override_.reasoning_effort;
            }
            if override_.max_output_tokens.is_some() {
                run.max_output_tokens = override_.max_output_tokens;
            }
            if let Some(prompt_refs) = &override_.prompt_refs {
                run.prompt_refs = prompt_refs.clone();
            }
            if let Some(tool_profile) = &override_.tool_profile {
                run.tool_profile = Some(tool_profile.clone());
            }
            if let Some(context_budget) = &override_.context_budget {
                run.context_budget = context_budget.clone();
            }
            if let Some(loop_limits) = &override_.loop_limits {
                run.loop_limits = loop_limits.clone();
            }
        }

        run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_json() {
        let config = SessionConfig {
            provider: "openai".into(),
            model: "gpt-x".into(),
            reasoning_effort: Some(ReasoningEffort::High),
            default_tool_profile: Some("local".into()),
            ..Default::default()
        };

        let encoded = serde_json::to_string(&config).expect("serialize config");
        let decoded: SessionConfig = serde_json::from_str(&encoded).expect("deserialize config");
        assert_eq!(decoded, config);
        assert!(decoded.extension_config_refs.is_empty());
    }

    #[test]
    fn run_config_override_resolves_from_session_defaults() {
        let session = SessionConfig {
            provider: "openai".into(),
            model: "default-model".into(),
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..Default::default()
        };
        let override_ = RunConfigOverride {
            model: Some("override-model".into()),
            reasoning_effort: Some(ReasoningEffort::High),
            ..Default::default()
        };

        let run = RunConfig::from_session(&session, Some(&override_));
        assert_eq!(run.provider, "openai");
        assert_eq!(run.model, "override-model");
        assert_eq!(run.reasoning_effort, Some(ReasoningEffort::High));
    }
}
