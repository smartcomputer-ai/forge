//! Live universe-environment discovery and session selection tool contracts.

use engine::{FunctionToolSpec, ToolKind, ToolName, ToolParallelism, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    error::{ToolError, ToolResult},
    runtime::{ToolBinding, ToolDispatchMode, ToolDocument, ToolSpecBundle},
};

pub const ENVIRONMENT_LIST_TOOL_NAME: &str = "environment_list";
pub const ENVIRONMENT_READ_TOOL_NAME: &str = "environment_read";
pub const ENVIRONMENT_ACTIVATE_TOOL_NAME: &str = "environment_activate";
pub const ENVIRONMENT_DEACTIVATE_TOOL_NAME: &str = "environment_deactivate";
pub const ENVIRONMENT_LOGICAL_ID_PREFIX: &str = "environment.";
pub const DEFAULT_ENVIRONMENT_LIST_LIMIT: usize = 20;
pub const MAX_ENVIRONMENT_LIST_LIMIT: usize = 100;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct EnvironmentListArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct EnvironmentReadArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct EnvironmentActivateArgs {
    pub environment_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDeactivateArgs {}

pub fn is_environment_control_tool(tool_name: &ToolName) -> bool {
    matches!(
        tool_name.as_str(),
        ENVIRONMENT_LIST_TOOL_NAME
            | ENVIRONMENT_READ_TOOL_NAME
            | ENVIRONMENT_ACTIVATE_TOOL_NAME
            | ENVIRONMENT_DEACTIVATE_TOOL_NAME
    )
}

pub fn is_environment_selection_tool(tool_name: &ToolName) -> bool {
    matches!(
        tool_name.as_str(),
        ENVIRONMENT_ACTIVATE_TOOL_NAME | ENVIRONMENT_DEACTIVATE_TOOL_NAME
    )
}

pub fn environment_control_tool_bundles(selection_tools: bool) -> ToolResult<Vec<ToolSpecBundle>> {
    let mut tools = vec![(
        ENVIRONMENT_READ_TOOL_NAME,
        "Read live details for an environment. Omit environment_id to inspect this session's active environment; provide a known id to inspect another environment allowed by the session.",
        optional_environment_id_schema(),
        ToolParallelism::ParallelSafe,
    )];
    if selection_tools {
        tools.extend([
            (
                ENVIRONMENT_LIST_TOOL_NAME,
                "List the live universe environments allowed by this session. Use this before activation when you do not know the environment id.",
                list_schema(),
                ToolParallelism::ParallelSafe,
            ),
            (
                ENVIRONMENT_ACTIVATE_TOOL_NAME,
                "Select one allowed, ready universe environment as this session's active environment. Environment-dependent tools must be called in a later turn.",
                required_environment_id_schema(),
                ToolParallelism::Exclusive,
            ),
            (
                ENVIRONMENT_DEACTIVATE_TOOL_NAME,
                "Clear this session's active environment without closing or changing the universe environment.",
                empty_schema(),
                ToolParallelism::Exclusive,
            ),
        ]);
    }
    tools
        .into_iter()
        .map(|(name, description, schema, parallelism)| {
            function_bundle(name, description, schema, parallelism)
        })
        .collect()
}

pub fn environment_control_tool_bindings(
    dispatch: ToolDispatchMode,
    selection_tools: bool,
) -> Vec<ToolBinding> {
    let mut tools = vec![(ENVIRONMENT_READ_TOOL_NAME, ToolParallelism::ParallelSafe)];
    if selection_tools {
        tools.extend([
            (ENVIRONMENT_LIST_TOOL_NAME, ToolParallelism::ParallelSafe),
            (ENVIRONMENT_ACTIVATE_TOOL_NAME, ToolParallelism::Exclusive),
            (ENVIRONMENT_DEACTIVATE_TOOL_NAME, ToolParallelism::Exclusive),
        ]);
    }
    tools
        .into_iter()
        .map(|(name, parallelism)| {
            ToolBinding::new(
                ToolName::new(name),
                format!(
                    "{ENVIRONMENT_LOGICAL_ID_PREFIX}{}",
                    name.trim_start_matches("environment_")
                ),
                dispatch.clone(),
                parallelism,
            )
        })
        .collect()
}

fn function_bundle(
    name: &'static str,
    description: &'static str,
    schema: Value,
    parallelism: ToolParallelism,
) -> ToolResult<ToolSpecBundle> {
    let description = ToolDocument::text("text/plain; charset=utf-8", description);
    let input_schema = ToolDocument::text(
        "application/schema+json",
        serde_json::to_string(&schema).map_err(|error| ToolError::InvalidRequest {
            message: format!("failed to encode {name} schema: {error}"),
        })?,
    );
    Ok(ToolSpecBundle {
        spec: ToolSpec {
            name: ToolName::new(name),
            kind: ToolKind::Function(FunctionToolSpec {
                description_ref: Some(description.blob_ref.clone()),
                input_schema_ref: input_schema.blob_ref.clone(),
                output_schema_ref: None,
                strict: Some(false),
                provider_options_ref: None,
            }),
            parallelism,
        },
        documents: vec![description, input_schema],
    })
}

fn list_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "cursor": { "type": ["string", "null"] },
            "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": MAX_ENVIRONMENT_LIST_LIMIT }
        },
        "additionalProperties": false
    })
}

fn optional_environment_id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "environment_id": { "type": ["string", "null"], "minLength": 1 }
        },
        "additionalProperties": false
    })
}

fn required_environment_id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "environment_id": { "type": "string", "minLength": 1 }
        },
        "required": ["environment_id"],
        "additionalProperties": false
    })
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_surface_separates_always_on_read_from_selection_tools() {
        let read_only = environment_control_tool_bundles(false).expect("read tool bundle");
        assert_eq!(read_only.len(), 1);
        assert_eq!(read_only[0].spec.name.as_str(), ENVIRONMENT_READ_TOOL_NAME);

        let bundles = environment_control_tool_bundles(true).expect("control tool bundles");
        assert_eq!(
            bundles
                .iter()
                .map(|bundle| bundle.spec.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                ENVIRONMENT_READ_TOOL_NAME,
                ENVIRONMENT_LIST_TOOL_NAME,
                ENVIRONMENT_ACTIVATE_TOOL_NAME,
                ENVIRONMENT_DEACTIVATE_TOOL_NAME,
            ]
        );
        assert_eq!(bundles[0].spec.parallelism, ToolParallelism::ParallelSafe);
        assert_eq!(bundles[1].spec.parallelism, ToolParallelism::ParallelSafe);
        assert_eq!(bundles[2].spec.parallelism, ToolParallelism::Exclusive);
        assert_eq!(bundles[3].spec.parallelism, ToolParallelism::Exclusive);
    }

    #[test]
    fn read_arguments_default_to_the_active_environment() {
        let args: EnvironmentReadArgs = serde_json::from_value(json!({})).expect("read args");
        assert_eq!(args.environment_id, None);
        let explicit: EnvironmentReadArgs =
            serde_json::from_value(json!({ "environment_id": "environment_1" }))
                .expect("explicit read args");
        assert_eq!(explicit.environment_id.as_deref(), Some("environment_1"));
    }

    #[test]
    fn selection_classification_excludes_discovery_tools() {
        assert!(!is_environment_selection_tool(&ToolName::new(
            ENVIRONMENT_LIST_TOOL_NAME,
        )));
        assert!(!is_environment_selection_tool(&ToolName::new(
            ENVIRONMENT_READ_TOOL_NAME,
        )));
        assert!(is_environment_selection_tool(&ToolName::new(
            ENVIRONMENT_ACTIVATE_TOOL_NAME,
        )));
        assert!(is_environment_selection_tool(&ToolName::new(
            ENVIRONMENT_DEACTIVATE_TOOL_NAME,
        )));
    }
}
