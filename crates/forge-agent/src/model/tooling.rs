//! Tool registry and tool-call planning records.
//!
//! This module will contain static tool specs, profiles, observed calls,
//! planned calls, and runtime context. Tool execution is implemented later.

use crate::ids::ToolCallId;
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ToolExecutorKind {
    #[default]
    Runner,
    Handler {
        handler_id: String,
    },
    Mcp {
        server: String,
        tool: String,
    },
    ProviderNative {
        provider: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ToolMapperKind {
    LlmPassthrough,
    JsonSchema,
    Mcp,
    ProviderNative,
    #[default]
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolParallelismHint {
    pub parallel_safe: bool,
    pub resource_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub tool_id: String,
    pub tool_name: String,
    pub description: String,
    pub args_schema: Value,
    pub mapper: ToolMapperKind,
    pub executor: ToolExecutorKind,
    pub parallelism_hint: ToolParallelismHint,
    pub definition_ref: Option<ArtifactRef>,
    pub estimated_tokens: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

impl ToolSpec {
    pub fn new(
        tool_id: impl Into<String>,
        tool_name: impl Into<String>,
        description: impl Into<String>,
        args_schema: Value,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            tool_name: tool_name.into(),
            description: description.into(),
            args_schema,
            mapper: ToolMapperKind::Custom,
            executor: ToolExecutorKind::Runner,
            parallelism_hint: ToolParallelismHint::default(),
            definition_ref: None,
            estimated_tokens: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolProfile {
    pub profile_id: String,
    pub tool_ids: Vec<String>,
    pub allow_parallel_results: bool,
    pub forced_tool_ids: Vec<String>,
    pub disabled_tool_ids: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

impl ToolProfile {
    pub fn selected_tool_ids(&self) -> Vec<String> {
        let disabled: BTreeSet<&String> = self.disabled_tool_ids.iter().collect();
        self.tool_ids
            .iter()
            .filter(|tool_id| !disabled.contains(tool_id))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRegistry {
    pub tools_by_id: BTreeMap<String, ToolSpec>,
    pub profiles: BTreeMap<String, ToolProfile>,
}

impl ToolRegistry {
    pub fn insert_tool(&mut self, tool: ToolSpec) -> Option<ToolSpec> {
        self.tools_by_id.insert(tool.tool_id.clone(), tool)
    }

    pub fn insert_profile(&mut self, profile: ToolProfile) -> Option<ToolProfile> {
        self.profiles.insert(profile.profile_id.clone(), profile)
    }

    pub fn tool_by_model_name(&self, tool_name: &str) -> Option<&ToolSpec> {
        self.tools_by_id
            .values()
            .find(|tool| tool.tool_name == tool_name)
    }

    pub fn model_visible_tools_for_profile(&self, profile_id: &str) -> Vec<ToolSpec> {
        let Some(profile) = self.profiles.get(profile_id) else {
            return Vec::new();
        };
        profile
            .selected_tool_ids()
            .iter()
            .filter_map(|tool_id| self.tools_by_id.get(tool_id))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRuntimeContext {
    pub active_capabilities: BTreeSet<String>,
    pub runtime_refs: Vec<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallObserved {
    pub call_id: ToolCallId,
    pub provider_call_id: Option<String>,
    pub tool_name: String,
    pub arguments_json: Option<String>,
    pub arguments_ref: Option<ArtifactRef>,
    pub raw_call_ref: Option<ArtifactRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedToolCall {
    pub call_id: ToolCallId,
    pub provider_call_id: Option<String>,
    pub tool_id: Option<String>,
    pub tool_name: String,
    pub arguments_json: Option<String>,
    pub arguments_ref: Option<ArtifactRef>,
    pub mapper: ToolMapperKind,
    pub executor: ToolExecutorKind,
    pub parallel_safe: bool,
    pub resource_key: Option<String>,
    pub accepted: bool,
    pub unavailable_reason: Option<String>,
}

impl PlannedToolCall {
    pub fn accepted(observed: &ToolCallObserved, tool: &ToolSpec) -> Self {
        Self {
            call_id: observed.call_id.clone(),
            provider_call_id: observed.provider_call_id.clone(),
            tool_id: Some(tool.tool_id.clone()),
            tool_name: tool.tool_name.clone(),
            arguments_json: observed.arguments_json.clone(),
            arguments_ref: observed.arguments_ref.clone(),
            mapper: tool.mapper.clone(),
            executor: tool.executor.clone(),
            parallel_safe: tool.parallelism_hint.parallel_safe,
            resource_key: tool.parallelism_hint.resource_key.clone(),
            accepted: true,
            unavailable_reason: None,
        }
    }

    pub fn unavailable(observed: &ToolCallObserved, reason: impl Into<String>) -> Self {
        Self {
            call_id: observed.call_id.clone(),
            provider_call_id: observed.provider_call_id.clone(),
            tool_id: None,
            tool_name: observed.tool_name.clone(),
            arguments_json: observed.arguments_json.clone(),
            arguments_ref: observed.arguments_ref.clone(),
            mapper: ToolMapperKind::Custom,
            executor: ToolExecutorKind::Runner,
            parallel_safe: false,
            resource_key: None,
            accepted: false,
            unavailable_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecutionGroup {
    pub call_ids: Vec<ToolCallId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecutionPlan {
    pub groups: Vec<ToolExecutionGroup>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolBatchPlan {
    pub observed_calls: Vec<ToolCallObserved>,
    pub planned_calls: Vec<PlannedToolCall>,
    pub execution_plan: ToolExecutionPlan,
}

impl ToolBatchPlan {
    pub fn from_planned_calls(
        observed_calls: Vec<ToolCallObserved>,
        planned_calls: Vec<PlannedToolCall>,
    ) -> Self {
        let execution_plan = ToolExecutionPlan::from_planned_calls(&planned_calls);
        Self {
            observed_calls,
            planned_calls,
            execution_plan,
        }
    }
}

impl ToolExecutionPlan {
    pub fn from_planned_calls(calls: &[PlannedToolCall]) -> Self {
        let mut groups = Vec::new();
        let mut current_group = Vec::new();
        let mut current_resources = BTreeSet::new();

        for call in calls.iter().filter(|call| call.accepted) {
            if !call.parallel_safe {
                flush_group(&mut groups, &mut current_group, &mut current_resources);
                groups.push(ToolExecutionGroup {
                    call_ids: vec![call.call_id.clone()],
                });
                continue;
            }

            if let Some(resource_key) = call.resource_key.as_ref() {
                if current_resources.contains(resource_key) {
                    flush_group(&mut groups, &mut current_group, &mut current_resources);
                }
                current_resources.insert(resource_key.clone());
            }
            current_group.push(call.call_id.clone());
        }

        flush_group(&mut groups, &mut current_group, &mut current_resources);
        Self { groups }
    }
}

fn flush_group(
    groups: &mut Vec<ToolExecutionGroup>,
    current_group: &mut Vec<ToolCallId>,
    current_resources: &mut BTreeSet<String>,
) {
    if current_group.is_empty() {
        return;
    }
    groups.push(ToolExecutionGroup {
        call_ids: std::mem::take(current_group),
    });
    current_resources.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_tool() -> ToolSpec {
        ToolSpec {
            tool_id: "read".into(),
            tool_name: "read_file".into(),
            description: "read".into(),
            args_schema: json!({"type":"object"}),
            parallelism_hint: ToolParallelismHint {
                parallel_safe: true,
                resource_key: Some("fs:/tmp/a".into()),
            },
            ..ToolSpec::default()
        }
    }

    #[test]
    fn tool_registry_selects_profile_tools_in_profile_order() {
        let mut registry = ToolRegistry::default();
        registry.insert_tool(read_tool());
        registry.insert_profile(ToolProfile {
            profile_id: "local".into(),
            tool_ids: vec!["read".into()],
            allow_parallel_results: true,
            ..Default::default()
        });

        let tools = registry.model_visible_tools_for_profile("local");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "read_file");
    }

    #[test]
    fn unavailable_planned_call_preserves_normalized_and_provider_ids() {
        let observed = ToolCallObserved {
            call_id: ToolCallId::new("call-1"),
            provider_call_id: Some("provider-call-1".into()),
            tool_name: "missing".into(),
            ..Default::default()
        };
        let planned = PlannedToolCall::unavailable(&observed, "unknown tool");

        assert!(!planned.accepted);
        assert_eq!(planned.call_id, ToolCallId::new("call-1"));
        assert_eq!(planned.provider_call_id.as_deref(), Some("provider-call-1"));
        assert_eq!(planned.unavailable_reason.as_deref(), Some("unknown tool"));
    }

    #[test]
    fn execution_plan_splits_resource_conflicts() {
        let first = PlannedToolCall {
            call_id: ToolCallId::new("call-1"),
            accepted: true,
            parallel_safe: true,
            resource_key: Some("fs:/same".into()),
            ..Default::default()
        };
        let second = PlannedToolCall {
            call_id: ToolCallId::new("call-2"),
            accepted: true,
            parallel_safe: true,
            resource_key: Some("fs:/same".into()),
            ..Default::default()
        };
        let third = PlannedToolCall {
            call_id: ToolCallId::new("call-3"),
            accepted: true,
            parallel_safe: false,
            ..Default::default()
        };

        let plan = ToolExecutionPlan::from_planned_calls(&[first, second, third]);
        assert_eq!(plan.groups.len(), 3);
        assert_eq!(plan.groups[0].call_ids, vec![ToolCallId::new("call-1")]);
        assert_eq!(plan.groups[1].call_ids, vec![ToolCallId::new("call-2")]);
        assert_eq!(plan.groups[2].call_ids, vec![ToolCallId::new("call-3")]);
    }
}
