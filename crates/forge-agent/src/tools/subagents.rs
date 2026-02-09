use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use crate::ToolError;

use super::{
    CLOSE_AGENT_TOOL, RegisteredTool, SEND_INPUT_TOOL, SPAWN_AGENT_TOOL, ToolExecutor, WAIT_TOOL,
};

pub(super) fn spawn_agent_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SPAWN_AGENT_TOOL.to_string(),
            description: "Spawn a subagent to handle a scoped task autonomously.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": { "type": "string" },
                    "working_dir": { "type": "string" },
                    "model": { "type": "string" },
                    "max_turns": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(SPAWN_AGENT_TOOL),
    }
}

pub(super) fn send_input_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SEND_INPUT_TOOL.to_string(),
            description: "Send a message to a running subagent.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id", "message"],
                "properties": {
                    "agent_id": { "type": "string" },
                    "message": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(SEND_INPUT_TOOL),
    }
}

pub(super) fn wait_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: WAIT_TOOL.to_string(),
            description: "Wait for a subagent to complete and return its result.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id"],
                "properties": {
                    "agent_id": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(WAIT_TOOL),
    }
}

pub(super) fn close_agent_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: CLOSE_AGENT_TOOL.to_string(),
            description: "Terminate a subagent.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id"],
                "properties": {
                    "agent_id": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(CLOSE_AGENT_TOOL),
    }
}

fn unsupported_subagent_executor(tool_name: &'static str) -> ToolExecutor {
    Arc::new(move |_args, _env| {
        Box::pin(async move {
            Err(ToolError::Execution(format!(
                "{} can only run inside a live Session dispatcher",
                tool_name
            ))
            .into())
        })
    })
}
