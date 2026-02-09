use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use crate::patch;

use super::{APPLY_PATCH_TOOL, RegisteredTool, required_string_argument};

pub(super) fn apply_patch_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: APPLY_PATCH_TOOL.to_string(),
            description: "Apply code changes using the patch format. Supports creating, deleting, and modifying files in a single operation.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["patch"],
                "properties": {
                    "patch": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let patch = required_string_argument(&args, "patch")?;
                let operations = patch::parse_apply_patch(&patch)?;
                patch::apply_patch_operations(&operations, env).await
            })
        }),
    }
}
