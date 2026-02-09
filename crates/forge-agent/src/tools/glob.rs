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

#[cfg(test)]
mod tests {
    use super::glob_tool;
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;

    struct GlobEnv;

    #[async_trait]
    impl ExecutionEnvironment for GlobEnv {
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
            Ok(vec!["a.txt".to_string(), "b.txt".to_string()])
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
    async fn glob_tool_joins_matches_with_newlines() {
        let tool = glob_tool();
        let env = Arc::new(GlobEnv);
        let output = (tool.executor)(json!({"pattern":"**/*.txt"}), env)
            .await
            .expect("executor should succeed");
        assert_eq!(output, "a.txt\nb.txt");
    }
}
