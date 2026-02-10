use super::{AgentError, EnvironmentContext, ProjectDocument, ProviderProfile};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct ModelOverrideProviderProfile {
    inner: Arc<dyn ProviderProfile>,
    model_override: String,
}

impl ModelOverrideProviderProfile {
    pub(super) fn new(inner: Arc<dyn ProviderProfile>, model_override: String) -> Self {
        Self {
            inner,
            model_override,
        }
    }
}

impl ProviderProfile for ModelOverrideProviderProfile {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn model(&self) -> &str {
        &self.model_override
    }

    fn tool_registry(&self) -> Arc<crate::ToolRegistry> {
        self.inner.tool_registry()
    }

    fn base_instructions(&self) -> &str {
        self.inner.base_instructions()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        self.inner.project_instruction_files()
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[forge_llm::ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        self.inner
            .build_system_prompt(environment, tools, project_docs, user_override)
    }

    fn tools(&self) -> Vec<forge_llm::ToolDefinition> {
        self.inner.tools()
    }

    fn provider_options(&self) -> Option<Value> {
        self.inner.provider_options()
    }

    fn capabilities(&self) -> crate::ProviderCapabilities {
        self.inner.capabilities()
    }

    fn knowledge_cutoff(&self) -> Option<&str> {
        self.inner.knowledge_cutoff()
    }
}

#[derive(Clone)]
pub(super) struct ScopedExecutionEnvironment {
    inner: Arc<dyn crate::ExecutionEnvironment>,
    scoped_working_directory: PathBuf,
    platform: String,
    os_version: String,
}

impl ScopedExecutionEnvironment {
    pub(super) fn new(
        inner: Arc<dyn crate::ExecutionEnvironment>,
        scoped_working_directory: PathBuf,
    ) -> Self {
        Self {
            platform: inner.platform().to_string(),
            os_version: inner.os_version().to_string(),
            inner,
            scoped_working_directory,
        }
    }

    fn resolve_path(&self, path: &str) -> String {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_string_lossy().to_string()
        } else {
            self.scoped_working_directory
                .join(candidate)
                .to_string_lossy()
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl crate::ExecutionEnvironment for ScopedExecutionEnvironment {
    async fn read_file(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, AgentError> {
        self.inner
            .read_file(&self.resolve_path(path), offset, limit)
            .await
    }

    async fn write_file(&self, path: &str, content: &str) -> Result<(), AgentError> {
        self.inner
            .write_file(&self.resolve_path(path), content)
            .await
    }

    async fn delete_file(&self, path: &str) -> Result<(), AgentError> {
        self.inner.delete_file(&self.resolve_path(path)).await
    }

    async fn move_file(&self, from: &str, to: &str) -> Result<(), AgentError> {
        self.inner
            .move_file(&self.resolve_path(from), &self.resolve_path(to))
            .await
    }

    async fn file_exists(&self, path: &str) -> Result<bool, AgentError> {
        self.inner.file_exists(&self.resolve_path(path)).await
    }

    async fn list_directory(
        &self,
        path: &str,
        depth: usize,
    ) -> Result<Vec<crate::DirEntry>, AgentError> {
        self.inner
            .list_directory(&self.resolve_path(path), depth)
            .await
    }

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<crate::ExecResult, AgentError> {
        let effective_working_dir = working_dir
            .map(|path| self.resolve_path(path))
            .unwrap_or_else(|| self.scoped_working_directory.to_string_lossy().to_string());
        self.inner
            .exec_command(command, timeout_ms, Some(&effective_working_dir), env_vars)
            .await
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: crate::GrepOptions,
    ) -> Result<String, AgentError> {
        self.inner
            .grep(pattern, &self.resolve_path(path), options)
            .await
    }

    async fn glob(&self, pattern: &str, path: &str) -> Result<Vec<String>, AgentError> {
        self.inner.glob(pattern, &self.resolve_path(path)).await
    }

    async fn initialize(&self) -> Result<(), AgentError> {
        self.inner.initialize().await
    }

    async fn cleanup(&self) -> Result<(), AgentError> {
        self.inner.cleanup().await
    }

    async fn terminate_all_commands(&self) -> Result<(), AgentError> {
        self.inner.terminate_all_commands().await
    }

    fn working_directory(&self) -> &Path {
        &self.scoped_working_directory
    }

    fn platform(&self) -> &str {
        &self.platform
    }

    fn os_version(&self) -> &str {
        &self.os_version
    }
}
