use crate::GrepOptions;
use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{
    GREP_TOOL, RegisteredTool, optional_bool_argument, optional_string_argument,
    optional_usize_argument, required_string_argument,
};

pub(super) fn grep_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: GREP_TOOL.to_string(),
            description: "Search file contents using regex patterns.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob_filter": { "type": "string" },
                    "case_insensitive": { "type": "boolean" },
                    "max_results": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let pattern = required_string_argument(&args, "pattern")?;
                let path = optional_string_argument(&args, "path")?.unwrap_or(".".to_string());
                let options = GrepOptions {
                    glob_filter: optional_string_argument(&args, "glob_filter")?,
                    case_insensitive: optional_bool_argument(&args, "case_insensitive")?
                        .unwrap_or(false),
                    max_results: optional_usize_argument(&args, "max_results")?.or(Some(100)),
                };

                let output = env.grep(&pattern, &path, options).await?;
                if output.trim().is_empty() {
                    Ok("No matches found".to_string())
                } else {
                    Ok(output)
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::grep_tool;
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct GrepEnv {
        path_seen: Mutex<Option<String>>,
    }

    #[async_trait]
    impl ExecutionEnvironment for GrepEnv {
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
            path: &str,
            _options: GrepOptions,
        ) -> Result<String, AgentError> {
            *self.path_seen.lock().expect("path mutex") = Some(path.to_string());
            Ok(String::new())
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
    async fn grep_tool_defaults_path_and_formats_empty_output() {
        let tool = grep_tool();
        let env = Arc::new(GrepEnv::default());
        let output = (tool.executor)(json!({"pattern":"abc"}), env.clone())
            .await
            .expect("executor should succeed");

        assert_eq!(output, "No matches found");
        assert_eq!(
            env.path_seen.lock().expect("path mutex").as_deref(),
            Some(".")
        );
    }
}
