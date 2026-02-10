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

#[cfg(test)]
mod tests {
    use super::{send_input_tool, spawn_agent_tool};
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;

    struct NoopEnv;

    #[async_trait]
    impl ExecutionEnvironment for NoopEnv {
        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("read_file".to_string()))
        }
        async fn write_file(&self, _path: &str, _content: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("write_file".to_string()))
        }
        async fn delete_file(&self, _path: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("delete_file".to_string()))
        }
        async fn move_file(&self, _from: &str, _to: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("move_file".to_string()))
        }
        async fn file_exists(&self, _path: &str) -> Result<bool, AgentError> {
            Err(AgentError::NotImplemented("file_exists".to_string()))
        }
        async fn list_directory(
            &self,
            _path: &str,
            _depth: usize,
        ) -> Result<Vec<crate::DirEntry>, AgentError> {
            Err(AgentError::NotImplemented("list_directory".to_string()))
        }
        async fn exec_command(
            &self,
            _command: &str,
            _timeout_ms: u64,
            _working_dir: Option<&str>,
            _env_vars: Option<HashMap<String, String>>,
        ) -> Result<crate::ExecResult, AgentError> {
            Err(AgentError::NotImplemented("exec_command".to_string()))
        }
        async fn grep(
            &self,
            _pattern: &str,
            _path: &str,
            _options: GrepOptions,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("grep".to_string()))
        }
        async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
            Err(AgentError::NotImplemented("glob".to_string()))
        }
        fn working_directory(&self) -> &Path {
            Path::new(".")
        }
        fn platform(&self) -> &str {
            "test"
        }
        fn os_version(&self) -> &str {
            "test"
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn subagent_executor_returns_session_only_error() {
        let tool = spawn_agent_tool();
        let err = (tool.executor)(json!({"task":"x"}), Arc::new(NoopEnv))
            .await
            .expect_err("executor should fail");
        assert!(
            err.to_string()
                .contains("can only run inside a live Session dispatcher")
        );
    }

    #[test]
    fn subagent_tool_definitions_include_required_fields() {
        let def = send_input_tool().definition;
        let required = def
            .parameters
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("required should be an array");
        let required_strings: Vec<&str> = required
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert!(required_strings.contains(&"agent_id"));
        assert!(required_strings.contains(&"message"));
    }
}
