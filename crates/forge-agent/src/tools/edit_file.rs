use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use crate::{ToolError, patch};

use super::{EDIT_FILE_TOOL, RegisteredTool, optional_bool_argument, required_string_argument};

pub(super) fn edit_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: EDIT_FILE_TOOL.to_string(),
            description: "Replace an exact string occurrence in a file.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path", "old_string", "new_string"],
                "properties": {
                    "file_path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let old_string = required_string_argument(&args, "old_string")?;
                let new_string = required_string_argument(&args, "new_string")?;
                let replace_all = optional_bool_argument(&args, "replace_all")?.unwrap_or(false);
                if old_string.is_empty() {
                    return Err(
                        ToolError::Execution("old_string must not be empty".to_string()).into(),
                    );
                }

                let content = env.read_file(&file_path, None, None).await?;
                let (next_content, replacement_count) =
                    patch::apply_edit(&content, &file_path, &old_string, &new_string, replace_all)?;
                env.write_file(&file_path, &next_content).await?;

                Ok(format!(
                    "Updated {} ({} replacement{})",
                    file_path,
                    replacement_count,
                    if replacement_count == 1 { "" } else { "s" }
                ))
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::edit_file_tool;
    use crate::{AgentError, ExecutionEnvironment, GrepOptions};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct EditEnv {
        content: Mutex<String>,
    }

    impl EditEnv {
        fn new(content: &str) -> Self {
            Self {
                content: Mutex::new(content.to_string()),
            }
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for EditEnv {
        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> Result<String, AgentError> {
            Ok(self.content.lock().expect("content mutex").clone())
        }
        async fn write_file(&self, _path: &str, content: &str) -> Result<(), AgentError> {
            *self.content.lock().expect("content mutex") = content.to_string();
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
    async fn edit_file_tool_applies_replacement() {
        let tool = edit_file_tool();
        let env = Arc::new(EditEnv::new("alpha\n"));
        let output = (tool.executor)(
            json!({"file_path":"f.txt","old_string":"alpha","new_string":"beta"}),
            env.clone(),
        )
        .await
        .expect("executor should succeed");

        assert!(output.contains("Updated f.txt"));
        assert_eq!(*env.content.lock().expect("content mutex"), "beta\n");
    }
}
