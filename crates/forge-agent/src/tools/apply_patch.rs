use forge_llm::ToolDefinition;
use serde_json::json;
use std::sync::Arc;

use crate::patch;

use super::{APPLY_PATCH_TOOL, RegisteredTool, required_string_argument};

pub(super) fn apply_patch_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: APPLY_PATCH_TOOL.to_string(),
            description: "Apply code changes using the patch format. Supports creating, deleting, and modifying files in a single operation.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["patch"],
                "properties": {
                    "patch": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let patch = required_string_argument(&args, "patch")?;
                let operations = patch::parse_apply_patch(&patch)?;
                patch::apply_patch_operations(&operations, env).await
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_patch_tool;
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
    async fn apply_patch_tool_surfaces_parse_errors() {
        let tool = apply_patch_tool();
        let env = Arc::new(NoopEnv);
        let err = (tool.executor)(
            json!({"patch":"*** Begin Patch\n*** End Patch\nextra"}),
            env,
        )
        .await
        .expect_err("executor should fail");
        assert!(err.to_string().contains("must end with '*** End Patch'"));
    }
}
