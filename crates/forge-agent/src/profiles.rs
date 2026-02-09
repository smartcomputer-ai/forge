use crate::ToolRegistry;
use forge_llm::ToolDefinition;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub supports_reasoning: bool,
    pub supports_streaming: bool,
    pub supports_parallel_tool_calls: bool,
    pub context_window_size: usize,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            supports_reasoning: true,
            supports_streaming: true,
            supports_parallel_tool_calls: false,
            context_window_size: 128_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentContext {
    pub working_directory: String,
    pub platform: String,
    pub os_version: String,
    pub is_git_repository: bool,
    pub git_branch: Option<String>,
    pub date_yyyy_mm_dd: String,
    pub model: String,
    pub knowledge_cutoff: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDocument {
    pub path: String,
    pub content: String,
}

pub trait ProviderProfile: Send + Sync {
    fn id(&self) -> &str;
    fn model(&self) -> &str;
    fn tool_registry(&self) -> Arc<ToolRegistry>;
    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        project_docs: &[ProjectDocument],
    ) -> String;
    fn tools(&self) -> Vec<ToolDefinition> {
        self.tool_registry().definitions()
    }
    fn provider_options(&self) -> Option<Value> {
        None
    }
    fn capabilities(&self) -> ProviderCapabilities;
}

#[derive(Clone)]
pub struct StaticProviderProfile {
    pub id: String,
    pub model: String,
    pub base_system_prompt: String,
    pub tool_registry: Arc<ToolRegistry>,
    pub provider_options: Option<Value>,
    pub capabilities: ProviderCapabilities,
}

impl ProviderProfile for StaticProviderProfile {
    fn id(&self) -> &str {
        &self.id
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    fn build_system_prompt(
        &self,
        _environment: &EnvironmentContext,
        _project_docs: &[ProjectDocument],
    ) -> String {
        self.base_system_prompt.clone()
    }

    fn provider_options(&self) -> Option<Value> {
        self.provider_options.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }
}
