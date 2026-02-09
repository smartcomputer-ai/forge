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

#[cfg(test)]
mod tests {
    use super::shell_tool;
    use crate::{AgentError, ExecResult, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct ShellEnv {
        timeout_seen: Mutex<Option<u64>>,
    }

    #[async_trait]
    impl ExecutionEnvironment for ShellEnv {
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
            timeout_ms: u64,
            _working_dir: Option<&str>,
            _env_vars: Option<HashMap<String, String>>,
        ) -> Result<ExecResult, AgentError> {
            *self.timeout_seen.lock().expect("timeout mutex") = Some(timeout_ms);
            Ok(ExecResult {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
                timed_out: false,
                duration_ms: 5,
            })
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
    async fn shell_tool_passes_timeout_and_formats_result() {
        let tool = shell_tool();
        let env = Arc::new(ShellEnv::default());
        let output = (tool.executor)(json!({"command":"echo hi","timeout_ms":42}), env.clone())
            .await
            .expect("executor should succeed");

        assert!(output.contains("exit_code: 0"));
        assert!(output.contains("stdout:"));
        assert_eq!(*env.timeout_seen.lock().expect("timeout mutex"), Some(42));
    }
}
