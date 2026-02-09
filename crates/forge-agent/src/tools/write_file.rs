use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use super::{RegisteredTool, WRITE_FILE_TOOL, required_string_argument};

pub(super) fn write_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: WRITE_FILE_TOOL.to_string(),
            description:
                "Write content to a file. Creates the file and parent directories if needed."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path", "content"],
                "properties": {
                    "file_path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let content = required_string_argument(&args, "content")?;
                env.write_file(&file_path, &content).await?;
                Ok(format!("Wrote {} bytes to {}", content.len(), file_path))
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::write_file_tool;
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct WriteEnv {
        write: Mutex<Option<(String, String)>>,
    }

    #[async_trait]
    impl ExecutionEnvironment for WriteEnv {
        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("read_file".to_string()))
        }
        async fn write_file(&self, path: &str, content: &str) -> Result<(), AgentError> {
            *self.write.lock().expect("write mutex") =
                Some((path.to_string(), content.to_string()));
            Ok(())
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
    async fn write_file_tool_writes_and_reports_bytes() {
        let tool = write_file_tool();
        let env = Arc::new(WriteEnv::default());
        let output = (tool.executor)(json!({"file_path":"f.txt","content":"abc"}), env.clone())
            .await
            .expect("executor should succeed");

        assert_eq!(output, "Wrote 3 bytes to f.txt");
        let write = env
            .write
            .lock()
            .expect("write mutex")
            .clone()
            .expect("write set");
        assert_eq!(write.0, "f.txt");
        assert_eq!(write.1, "abc");
    }
}
