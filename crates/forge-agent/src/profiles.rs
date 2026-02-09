use crate::ToolRegistry;
use forge_llm::ToolDefinition;
use serde_json::Value;
use std::sync::Arc;

pub const OPENAI_PROFILE_ID: &str = "openai";
pub const ANTHROPIC_PROFILE_ID: &str = "anthropic";
pub const GEMINI_PROFILE_ID: &str = "gemini";

pub const PROJECT_DOC_TRUNCATION_MARKER: &str = "[Project instructions truncated at 32KB]";

const DEFAULT_OPENAI_INSTRUCTIONS: &str = "\
You are a coding agent running in Forge (OpenAI profile).
Follow instructions in this order: system, project docs, user request.
Use tools deliberately, prefer minimally scoped reads, and keep edits precise.
When applying patches, keep hunks minimal and verify file paths before writing.
After edits or commands, summarize what changed and surface important risks.";

const DEFAULT_ANTHROPIC_INSTRUCTIONS: &str = "\
You are a coding agent running in Forge (Anthropic profile).
Read relevant files before editing and prefer targeted edits over full rewrites.
Use edit_file with exact old_string/new_string matches and avoid ambiguous replacements.
Use shell commands only when necessary and explain failures with actionable next steps.
After changes, report what you validated and what still needs verification.";

const DEFAULT_GEMINI_INSTRUCTIONS: &str = "\
You are a coding agent running in Forge (Gemini profile).
Use tools intentionally, keep steps explicit, and avoid speculative edits.
Honor GEMINI.md and AGENTS.md project conventions when present.
Prefer concise, deterministic changes and include validation outcomes.
If blocked, state the blocker clearly and propose the next concrete action.";

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
    pub repository_root: Option<String>,
    pub platform: String,
    pub os_version: String,
    pub is_git_repository: bool,
    pub git_branch: Option<String>,
    pub git_status_summary: Option<String>,
    pub git_recent_commits: Vec<String>,
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
    fn base_instructions(&self) -> &str;
    fn project_instruction_files(&self) -> Vec<String> {
        default_project_instruction_files_for_profile(self.id())
    }
    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String;
    fn tools(&self) -> Vec<ToolDefinition> {
        self.tool_registry().definitions()
    }
    fn provider_options(&self) -> Option<Value> {
        None
    }
    fn capabilities(&self) -> ProviderCapabilities;
    fn knowledge_cutoff(&self) -> Option<&str> {
        None
    }
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

    fn base_instructions(&self) -> &str {
        &self.base_system_prompt
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        build_layered_system_prompt(
            self.base_instructions(),
            environment,
            tools,
            project_docs,
            user_override,
        )
    }

    fn provider_options(&self) -> Option<Value> {
        self.provider_options.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        default_project_instruction_files_for_profile(&self.id)
    }
}

#[derive(Clone)]
pub struct OpenAiProviderProfile {
    model: String,
    tool_registry: Arc<ToolRegistry>,
    provider_options: Option<Value>,
    capabilities: ProviderCapabilities,
    base_instructions: String,
    knowledge_cutoff: Option<String>,
}

impl OpenAiProviderProfile {
    pub fn new(model: impl Into<String>, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            model: model.into(),
            tool_registry,
            provider_options: None,
            capabilities: ProviderCapabilities {
                supports_parallel_tool_calls: true,
                context_window_size: 200_000,
                ..ProviderCapabilities::default()
            },
            base_instructions: DEFAULT_OPENAI_INSTRUCTIONS.to_string(),
            knowledge_cutoff: None,
        }
    }

    pub fn with_provider_options(mut self, provider_options: Value) -> Self {
        self.provider_options = Some(provider_options);
        self
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_base_instructions(mut self, base_instructions: impl Into<String>) -> Self {
        self.base_instructions = base_instructions.into();
        self
    }

    pub fn with_knowledge_cutoff(mut self, knowledge_cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(knowledge_cutoff.into());
        self
    }
}

impl ProviderProfile for OpenAiProviderProfile {
    fn id(&self) -> &str {
        OPENAI_PROFILE_ID
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    fn base_instructions(&self) -> &str {
        &self.base_instructions
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        build_layered_system_prompt(
            self.base_instructions(),
            environment,
            tools,
            project_docs,
            user_override,
        )
    }

    fn provider_options(&self) -> Option<Value> {
        self.provider_options.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn knowledge_cutoff(&self) -> Option<&str> {
        self.knowledge_cutoff.as_deref()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        vec![
            "AGENTS.md".to_string(),
            ".codex/instructions.md".to_string(),
        ]
    }
}

#[derive(Clone)]
pub struct AnthropicProviderProfile {
    model: String,
    tool_registry: Arc<ToolRegistry>,
    provider_options: Option<Value>,
    capabilities: ProviderCapabilities,
    base_instructions: String,
    knowledge_cutoff: Option<String>,
}

impl AnthropicProviderProfile {
    pub fn new(model: impl Into<String>, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            model: model.into(),
            tool_registry,
            provider_options: None,
            capabilities: ProviderCapabilities {
                supports_parallel_tool_calls: true,
                context_window_size: 200_000,
                ..ProviderCapabilities::default()
            },
            base_instructions: DEFAULT_ANTHROPIC_INSTRUCTIONS.to_string(),
            knowledge_cutoff: None,
        }
    }

    pub fn with_provider_options(mut self, provider_options: Value) -> Self {
        self.provider_options = Some(provider_options);
        self
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_base_instructions(mut self, base_instructions: impl Into<String>) -> Self {
        self.base_instructions = base_instructions.into();
        self
    }

    pub fn with_knowledge_cutoff(mut self, knowledge_cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(knowledge_cutoff.into());
        self
    }
}

impl ProviderProfile for AnthropicProviderProfile {
    fn id(&self) -> &str {
        ANTHROPIC_PROFILE_ID
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    fn base_instructions(&self) -> &str {
        &self.base_instructions
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        build_layered_system_prompt(
            self.base_instructions(),
            environment,
            tools,
            project_docs,
            user_override,
        )
    }

    fn provider_options(&self) -> Option<Value> {
        self.provider_options.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn knowledge_cutoff(&self) -> Option<&str> {
        self.knowledge_cutoff.as_deref()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
    }
}

#[derive(Clone)]
pub struct GeminiProviderProfile {
    model: String,
    tool_registry: Arc<ToolRegistry>,
    provider_options: Option<Value>,
    capabilities: ProviderCapabilities,
    base_instructions: String,
    knowledge_cutoff: Option<String>,
}

impl GeminiProviderProfile {
    pub fn new(model: impl Into<String>, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            model: model.into(),
            tool_registry,
            provider_options: None,
            capabilities: ProviderCapabilities {
                supports_parallel_tool_calls: true,
                context_window_size: 1_000_000,
                ..ProviderCapabilities::default()
            },
            base_instructions: DEFAULT_GEMINI_INSTRUCTIONS.to_string(),
            knowledge_cutoff: None,
        }
    }

    pub fn with_provider_options(mut self, provider_options: Value) -> Self {
        self.provider_options = Some(provider_options);
        self
    }

    pub fn with_capabilities(mut self, capabilities: ProviderCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_base_instructions(mut self, base_instructions: impl Into<String>) -> Self {
        self.base_instructions = base_instructions.into();
        self
    }

    pub fn with_knowledge_cutoff(mut self, knowledge_cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(knowledge_cutoff.into());
        self
    }
}

impl ProviderProfile for GeminiProviderProfile {
    fn id(&self) -> &str {
        GEMINI_PROFILE_ID
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn tool_registry(&self) -> Arc<ToolRegistry> {
        self.tool_registry.clone()
    }

    fn base_instructions(&self) -> &str {
        &self.base_instructions
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        build_layered_system_prompt(
            self.base_instructions(),
            environment,
            tools,
            project_docs,
            user_override,
        )
    }

    fn provider_options(&self) -> Option<Value> {
        self.provider_options.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    fn knowledge_cutoff(&self) -> Option<&str> {
        self.knowledge_cutoff.as_deref()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        vec!["AGENTS.md".to_string(), "GEMINI.md".to_string()]
    }
}

pub fn default_project_instruction_files_for_profile(profile_id: &str) -> Vec<String> {
    let mut files = vec!["AGENTS.md".to_string()];
    match profile_id {
        OPENAI_PROFILE_ID => files.push(".codex/instructions.md".to_string()),
        ANTHROPIC_PROFILE_ID => files.push("CLAUDE.md".to_string()),
        GEMINI_PROFILE_ID => files.push("GEMINI.md".to_string()),
        _ => {}
    }
    files
}

pub fn build_layered_system_prompt(
    base_instructions: &str,
    environment: &EnvironmentContext,
    tools: &[ToolDefinition],
    project_docs: &[ProjectDocument],
    user_override: Option<&str>,
) -> String {
    let mut layers = vec![
        format!(
            "## Provider Base Instructions\n{}",
            base_instructions.trim()
        ),
        format_environment_context_block(environment),
        format_tool_descriptions_block(tools),
        format_project_docs_block(project_docs),
    ];

    if let Some(override_text) = user_override {
        let override_text = override_text.trim();
        if !override_text.is_empty() {
            layers.push(format!(
                "## User Instructions Override (Highest Priority)\n{}",
                override_text
            ));
        }
    }

    layers.join("\n\n")
}

fn format_environment_context_block(environment: &EnvironmentContext) -> String {
    let repository_root = environment
        .repository_root
        .as_deref()
        .unwrap_or("n/a")
        .to_string();
    let git_branch = environment
        .git_branch
        .as_deref()
        .unwrap_or("n/a")
        .to_string();
    let git_status_summary = environment
        .git_status_summary
        .as_deref()
        .unwrap_or("n/a")
        .to_string();
    let knowledge_cutoff = environment
        .knowledge_cutoff
        .as_deref()
        .unwrap_or("unknown")
        .to_string();
    let commits = if environment.git_recent_commits.is_empty() {
        "none".to_string()
    } else {
        environment.git_recent_commits.join(" | ")
    };

    format!(
        "<environment>\nWorking directory: {}\nRepository root: {}\nIs git repository: {}\nGit branch: {}\nGit status summary: {}\nRecent commits: {}\nPlatform: {}\nOS version: {}\nToday's date: {}\nModel: {}\nKnowledge cutoff: {}\n</environment>",
        environment.working_directory,
        repository_root,
        environment.is_git_repository,
        git_branch,
        git_status_summary,
        commits,
        environment.platform,
        environment.os_version,
        environment.date_yyyy_mm_dd,
        environment.model,
        knowledge_cutoff
    )
}

fn format_tool_descriptions_block(tools: &[ToolDefinition]) -> String {
    if tools.is_empty() {
        return "<tools>\n(none)\n</tools>".to_string();
    }

    let mut lines = vec!["<tools>".to_string()];
    for tool in tools {
        lines.push(format!("- {}: {}", tool.name, tool.description));
    }
    lines.push("</tools>".to_string());
    lines.join("\n")
}

fn format_project_docs_block(project_docs: &[ProjectDocument]) -> String {
    if project_docs.is_empty() {
        return "<project_instructions>\n(none)\n</project_instructions>".to_string();
    }

    let mut lines = vec!["<project_instructions>".to_string()];
    for doc in project_docs {
        lines.push(format!("### {}", doc.path));
        lines.push(doc.content.clone());
    }
    lines.push("</project_instructions>".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RegisteredTool, ToolExecutor};
    use serde_json::json;

    fn dummy_environment() -> EnvironmentContext {
        EnvironmentContext {
            working_directory: "/repo/work".to_string(),
            repository_root: Some("/repo".to_string()),
            platform: "linux".to_string(),
            os_version: "linux-test".to_string(),
            is_git_repository: true,
            git_branch: Some("main".to_string()),
            git_status_summary: Some("modified: 2, untracked: 1".to_string()),
            git_recent_commits: vec![
                "abc123 add tests".to_string(),
                "def456 tighten prompt layering".to_string(),
            ],
            date_yyyy_mm_dd: "2026-02-09".to_string(),
            model: "gpt-5.2-codex".to_string(),
            knowledge_cutoff: Some("2024-10".to_string()),
        }
    }

    #[test]
    fn default_project_instruction_files_are_profile_specific() {
        assert_eq!(
            default_project_instruction_files_for_profile(OPENAI_PROFILE_ID),
            vec![
                "AGENTS.md".to_string(),
                ".codex/instructions.md".to_string()
            ]
        );
        assert_eq!(
            default_project_instruction_files_for_profile(ANTHROPIC_PROFILE_ID),
            vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
        );
        assert_eq!(
            default_project_instruction_files_for_profile(GEMINI_PROFILE_ID),
            vec!["AGENTS.md".to_string(), "GEMINI.md".to_string()]
        );
        assert_eq!(
            default_project_instruction_files_for_profile("custom"),
            vec!["AGENTS.md".to_string()]
        );
    }

    #[test]
    fn build_layered_system_prompt_orders_layers_deterministically() {
        let mut registry = ToolRegistry::default();
        let no_op: ToolExecutor = Arc::new(|_, _| Box::pin(async { Ok(String::new()) }));
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "zeta".to_string(),
                description: "last tool".to_string(),
                parameters: json!({"type":"object"}),
            },
            executor: no_op.clone(),
        });
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "alpha".to_string(),
                description: "first tool".to_string(),
                parameters: json!({"type":"object"}),
            },
            executor: no_op,
        });
        let profile = StaticProviderProfile {
            id: OPENAI_PROFILE_ID.to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "Base prompt".to_string(),
            tool_registry: Arc::new(registry),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        };
        let docs = vec![ProjectDocument {
            path: "AGENTS.md".to_string(),
            content: "Be precise".to_string(),
        }];

        let prompt = profile.build_system_prompt(
            &dummy_environment(),
            &profile.tools(),
            &docs,
            Some("Always run tests"),
        );

        let base_idx = prompt
            .find("## Provider Base Instructions")
            .expect("base layer should exist");
        let env_idx = prompt
            .find("<environment>")
            .expect("environment layer should exist");
        let tools_idx = prompt.find("<tools>").expect("tools layer should exist");
        let docs_idx = prompt
            .find("<project_instructions>")
            .expect("project docs layer should exist");
        let override_idx = prompt
            .find("## User Instructions Override (Highest Priority)")
            .expect("override layer should exist");
        assert!(base_idx < env_idx);
        assert!(env_idx < tools_idx);
        assert!(tools_idx < docs_idx);
        assert!(docs_idx < override_idx);

        let alpha_idx = prompt
            .find("- alpha: first tool")
            .expect("alpha tool listed");
        let zeta_idx = prompt.find("- zeta: last tool").expect("zeta tool listed");
        assert!(alpha_idx < zeta_idx);
    }
}
