use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{READ_FILE_TOOL, RegisteredTool, required_string_argument};

pub(super) fn read_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: READ_FILE_TOOL.to_string(),
            description: "Read a file from the filesystem. Returns line-numbered content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path"],
                "properties": {
                    "file_path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let offset = super::optional_usize_argument(&args, "offset")?;
                let limit = super::optional_usize_argument(&args, "limit")?;

                let content = env.read_file(&file_path, offset, limit).await?;
                Ok(super::format_line_numbered_content(
                    &content,
                    offset.unwrap_or(1),
                ))
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::read_file_tool;
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct ReadEnv {
        call: Mutex<Option<(String, Option<usize>, Option<usize>)>>,
    }

    #[async_trait]
    impl ExecutionEnvironment for ReadEnv {
        async fn read_file(
            &self,
            path: &str,
            offset: Option<usize>,
            limit: Option<usize>,
        ) -> Result<String, AgentError> {
            *self.call.lock().expect("call mutex") = Some((path.to_string(), offset, limit));
            Ok("alpha\nbeta".to_string())
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
    async fn read_file_tool_formats_line_numbered_output() {
        let tool = read_file_tool();
        let env = Arc::new(ReadEnv::default());
        let output = (tool.executor)(
            json!({"file_path":"a.txt","offset":2,"limit":2}),
            env.clone(),
        )
        .await
        .expect("executor should succeed");

        assert_eq!(output, "2 | alpha\n3 | beta");
        let call = env
            .call
            .lock()
            .expect("call mutex")
            .clone()
            .expect("call set");
        assert_eq!(call.0, "a.txt");
        assert_eq!(call.1, Some(2));
        assert_eq!(call.2, Some(2));
    }
}
