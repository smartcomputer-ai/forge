use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{RegisteredTool, SHELL_TOOL, optional_u64_argument, required_string_argument};

pub(super) fn shell_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SHELL_TOOL.to_string(),
            description: "Execute a shell command. Returns stdout, stderr, and exit code."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "description": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let command = required_string_argument(&args, "command")?;
                let timeout_ms = optional_u64_argument(&args, "timeout_ms")?.unwrap_or(0);
                let result = env.exec_command(&command, timeout_ms, None, None).await?;
                Ok(super::format_exec_result(&result))
            })
        }),
    }
}
