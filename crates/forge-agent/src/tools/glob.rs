use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{GLOB_TOOL, RegisteredTool, optional_string_argument, required_string_argument};

pub(super) fn glob_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: GLOB_TOOL.to_string(),
            description: "Find files matching a glob pattern.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let pattern = required_string_argument(&args, "pattern")?;
                let path = optional_string_argument(&args, "path")?.unwrap_or(".".to_string());
                let matches = env.glob(&pattern, &path).await?;
                if matches.is_empty() {
                    Ok("No files matched".to_string())
                } else {
                    Ok(matches.join("\n"))
                }
            })
        }),
    }
}
