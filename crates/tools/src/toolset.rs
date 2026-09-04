//! Provider-aware session toolset composition.

use std::collections::{BTreeMap, BTreeSet};

use engine::{ProviderApiKind, ToolName, ToolSpec, WorkflowToolBinding};

use crate::{
    builtin::{BuiltinTool, BuiltinToolOperation, BuiltinToolSurface},
    concurrency::{ConcurrencyToolsetConfig, concurrency_tool_bindings, concurrency_tool_bundles},
    environment::control::{environment_control_tool_bindings, environment_control_tool_bundles},
    error::{ToolError, ToolResult},
    runtime::{ToolCatalog, ToolDispatchMode, ToolDocument, ToolSpecBundle, ToolTarget},
    web::fetch::{
        WebFetchToolConfig, anthropic_messages_web_fetch_tool_bundle, web_fetch_tool_binding,
        web_fetch_tool_bundle,
    },
    web::search::{
        OpenAiResponsesWebSearchConfig, WebSearchMode, WebSearchToolConfig,
        anthropic_messages_web_search_tool_bundle, apply_openai_responses_web_search_includes,
        openai_responses_web_search_tool_bundle,
    },
    workflow_tool::workflow_tool_tool_binding,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolsetConfig {
    pub builtin: BuiltinToolsetConfig,
    pub web: WebToolsetConfig,
    pub concurrency: ConcurrencyToolsetConfig,
    pub environment_read: bool,
    pub environment_selection: bool,
}

impl ToolsetConfig {
    pub fn empty() -> Self {
        Self {
            builtin: BuiltinToolsetConfig::disabled(),
            web: WebToolsetConfig::default(),
            concurrency: ConcurrencyToolsetConfig::default(),
            environment_read: false,
            environment_selection: false,
        }
    }

    pub fn workspace() -> Self {
        Self {
            builtin: BuiltinToolsetConfig::workspace(),
            ..Self::empty()
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WebToolsetConfig {
    pub search: Option<WebSearchToolConfig>,
    pub fetch: bool,
}

impl Default for ToolsetConfig {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltinToolsetConfig {
    pub presentation: BuiltinToolPresentation,
    pub vfs: FilesystemToolsetConfig,
    pub environment: EnvironmentToolsetConfig,
    pub dispatch: ToolDispatchMode,
}

impl BuiltinToolsetConfig {
    pub fn disabled() -> Self {
        Self {
            presentation: BuiltinToolPresentation::ProviderDefault,
            vfs: FilesystemToolsetConfig::disabled(),
            environment: EnvironmentToolsetConfig::disabled(),
            dispatch: ToolDispatchMode::Local,
        }
    }

    pub fn workspace() -> Self {
        Self {
            vfs: FilesystemToolsetConfig::workspace_edit(),
            ..Self::disabled()
        }
    }

    pub fn from_operations(operations: impl IntoIterator<Item = BuiltinToolOperation>) -> Self {
        let mut config = Self::disabled();
        for operation in operations {
            config.enable_operation(operation);
        }
        config
    }

    pub fn vfs_from_operations(
        operations: impl IntoIterator<Item = BuiltinToolOperation>,
    ) -> ToolResult<Self> {
        let mut config = Self::disabled();
        for operation in operations {
            match operation {
                BuiltinToolOperation::ReadFile => config.vfs.read_file = true,
                BuiltinToolOperation::WriteFile => config.vfs.write_file = true,
                BuiltinToolOperation::EditFile => config.vfs.edit_file = true,
                BuiltinToolOperation::ApplyPatch => config.vfs.apply_patch = true,
                BuiltinToolOperation::Grep => config.vfs.grep = true,
                BuiltinToolOperation::Glob => config.vfs.glob = true,
                BuiltinToolOperation::ListDir => config.vfs.list_dir = true,
                BuiltinToolOperation::RunProcess
                | BuiltinToolOperation::ContinueProcess
                | BuiltinToolOperation::JobSubmit
                | BuiltinToolOperation::JobRun
                | BuiltinToolOperation::JobRead => {
                    return Err(ToolError::InvalidRequest {
                        message: "non-filesystem operation cannot be enabled in the VFS domain"
                            .to_owned(),
                    });
                }
            }
        }
        Ok(config)
    }

    pub fn enable_operation(&mut self, operation: BuiltinToolOperation) {
        match operation {
            BuiltinToolOperation::ReadFile => self.environment.filesystem.read_file = true,
            BuiltinToolOperation::WriteFile => self.environment.filesystem.write_file = true,
            BuiltinToolOperation::EditFile => self.environment.filesystem.edit_file = true,
            BuiltinToolOperation::ApplyPatch => self.environment.filesystem.apply_patch = true,
            BuiltinToolOperation::Grep => self.environment.filesystem.grep = true,
            BuiltinToolOperation::Glob => self.environment.filesystem.glob = true,
            BuiltinToolOperation::ListDir => self.environment.filesystem.list_dir = true,
            BuiltinToolOperation::RunProcess => self.environment.run_process = true,
            BuiltinToolOperation::ContinueProcess => self.environment.continue_process = true,
            BuiltinToolOperation::JobSubmit => self.environment.job_submit = true,
            BuiltinToolOperation::JobRun => self.environment.job_run = true,
            BuiltinToolOperation::JobRead => self.environment.job_read = true,
        }
    }

    pub fn enabled(&self) -> bool {
        self.vfs.enabled() || self.environment.enabled()
    }
}

impl Default for BuiltinToolsetConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuiltinToolPresentation {
    #[default]
    ProviderDefault,
    Canonical,
    CodexLike,
    ClaudeCodeLike,
}

impl BuiltinToolPresentation {
    /// The provider default gives OpenAI and Anthropic models the shapes
    /// their harnesses trained them on. OpenAI Completions, the
    /// compatibility API most other providers speak, gets the neutral
    /// canonical surface.
    fn surface(self, target: &ToolTarget) -> BuiltinToolSurface {
        match self {
            Self::ProviderDefault => match target.api_kind {
                ProviderApiKind::AnthropicMessages => BuiltinToolSurface::ClaudeCodeLike,
                ProviderApiKind::OpenAiResponses => BuiltinToolSurface::CodexLike,
                ProviderApiKind::OpenAiCompletions => BuiltinToolSurface::Canonical,
            },
            Self::Canonical => BuiltinToolSurface::Canonical,
            Self::CodexLike => BuiltinToolSurface::CodexLike,
            Self::ClaudeCodeLike => BuiltinToolSurface::ClaudeCodeLike,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilesystemToolsetConfig {
    pub read_file: bool,
    pub write_file: bool,
    pub edit_file: bool,
    pub apply_patch: bool,
    pub grep: bool,
    pub glob: bool,
    pub list_dir: bool,
}

impl FilesystemToolsetConfig {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn read_only() -> Self {
        Self {
            read_file: true,
            grep: true,
            glob: true,
            list_dir: true,
            ..Self::disabled()
        }
    }

    pub fn workspace_edit() -> Self {
        Self {
            write_file: true,
            edit_file: true,
            apply_patch: true,
            ..Self::read_only()
        }
    }

    pub fn enabled(&self) -> bool {
        self.read_file
            || self.write_file
            || self.edit_file
            || self.apply_patch
            || self.grep
            || self.glob
            || self.list_dir
    }

    fn operations(&self) -> Vec<BuiltinToolOperation> {
        let mut operations = Vec::new();
        if self.read_file {
            operations.push(BuiltinToolOperation::ReadFile);
        }
        if self.write_file {
            operations.push(BuiltinToolOperation::WriteFile);
        }
        if self.edit_file {
            operations.push(BuiltinToolOperation::EditFile);
        }
        if self.apply_patch {
            operations.push(BuiltinToolOperation::ApplyPatch);
        }
        if self.grep {
            operations.push(BuiltinToolOperation::Grep);
        }
        if self.glob {
            operations.push(BuiltinToolOperation::Glob);
        }
        if self.list_dir {
            operations.push(BuiltinToolOperation::ListDir);
        }
        operations
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvironmentToolsetConfig {
    pub filesystem: FilesystemToolsetConfig,
    pub run_process: bool,
    /// The handle path. When off, every surface renders its one-shot
    /// process shape: the run tool waits to exit and no continue tool exists.
    pub continue_process: bool,
    pub job_submit: bool,
    pub job_run: bool,
    pub job_read: bool,
}

impl EnvironmentToolsetConfig {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn basic() -> Self {
        Self {
            filesystem: FilesystemToolsetConfig::workspace_edit(),
            run_process: true,
            continue_process: true,
            ..Self::disabled()
        }
    }

    pub fn jobs() -> Self {
        Self {
            job_submit: true,
            job_read: true,
            ..Self::disabled()
        }
    }

    pub fn with_jobs(mut self) -> Self {
        self.job_submit = true;
        self.job_read = true;
        self
    }

    pub fn enabled(&self) -> bool {
        self.filesystem.enabled()
            || self.run_process
            || self.continue_process
            || self.job_submit
            || self.job_run
            || self.job_read
    }

    pub fn jobs_enabled(&self) -> bool {
        self.job_submit || self.job_run || self.job_read
    }

    fn operations(&self) -> Vec<BuiltinToolOperation> {
        let mut operations = self.filesystem.operations();
        if self.run_process {
            operations.push(BuiltinToolOperation::RunProcess);
        }
        if self.run_process && self.continue_process {
            operations.push(BuiltinToolOperation::ContinueProcess);
        }
        if self.job_submit {
            operations.push(BuiltinToolOperation::JobSubmit);
        }
        if self.job_run {
            operations.push(BuiltinToolOperation::JobRun);
        }
        if self.job_read {
            operations.push(BuiltinToolOperation::JobRead);
        }
        operations
    }
}

pub struct ToolsetEnvironment<'a> {
    pub target: &'a ToolTarget,
}

/// Provider request parameter additions required by the resolved toolset.
///
/// The toolset only reports the required values; applying them to a session's
/// opaque provider params is owned by the runtime layer that knows the params
/// schema.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderParamsPatch {
    openai_responses_include: Vec<String>,
}

impl ProviderParamsPatch {
    pub fn is_empty(&self) -> bool {
        self.openai_responses_include.is_empty()
    }

    /// OpenAI Responses `include` values the toolset needs on generation
    /// requests.
    pub fn openai_responses_include(&self) -> &[String] {
        &self.openai_responses_include
    }

    fn add_openai_web_search(&mut self, config: &OpenAiResponsesWebSearchConfig) {
        let mut include = Vec::new();
        apply_openai_responses_web_search_includes(&mut include, config);
        for value in include {
            if !self
                .openai_responses_include
                .iter()
                .any(|existing| existing == &value)
            {
                self.openai_responses_include.push(value);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedToolset {
    pub tools: BTreeMap<ToolName, ToolSpec>,
    pub documents: Vec<ToolDocument>,
    pub catalog: ToolCatalog,
    pub provider_params_patch: ProviderParamsPatch,
}

/// Explicit-Promise workflow tools create model-owned completion promises, so their
/// admission pulls concurrency tools into the model toolset. Joined tools use
/// runtime-owned Promises and must not grant await/cancel/detach.
pub fn enable_concurrency_for_workflow_tools<'a>(
    config: &mut ToolsetConfig,
    bindings: impl IntoIterator<Item = &'a WorkflowToolBinding>,
) {
    if bindings
        .into_iter()
        .any(|binding| binding.completion.exposes_model_owned_promises())
    {
        config.concurrency.enabled = true;
    }
}

/// Merge trusted, already-admitted workflow tools into the effective
/// provider-facing toolset and runtime catalog.
pub fn materialize_workflow_tools<'a>(
    toolset: &mut ResolvedToolset,
    bindings: impl IntoIterator<Item = &'a WorkflowToolBinding>,
) -> ToolResult<()> {
    for binding in bindings {
        binding
            .validate()
            .map_err(|error| ToolError::InvalidRequest {
                message: format!(
                    "invalid workflow tool {} binding: {error}",
                    binding.definition.tool_id
                ),
            })?;
        let tool_name = binding.definition.tool.name.clone();
        if toolset.tools.contains_key(&tool_name) {
            return Err(ToolError::InvalidRequest {
                message: format!(
                    "workflow tool {} tool name {} collides with another effective tool",
                    binding.definition.tool_id, tool_name
                ),
            });
        }
        toolset
            .tools
            .insert(tool_name, binding.definition.tool.clone());
        toolset.catalog.insert(workflow_tool_tool_binding(binding));
    }
    Ok(())
}

pub fn resolve_toolset(
    env: ToolsetEnvironment<'_>,
    config: &ToolsetConfig,
) -> ToolResult<ResolvedToolset> {
    let mut builder = ToolsetBuilder::new();

    if config.builtin.enabled() {
        builder.add_builtin_tools(env.target, &config.builtin)?;
    }

    if let Some(search) = &config.web.search {
        let bundle = match env.target.api_kind {
            ProviderApiKind::OpenAiResponses => {
                let provider_config = OpenAiResponsesWebSearchConfig {
                    mode: WebSearchMode::Cached,
                    allowed_domains: search.allowed_domains.clone(),
                    blocked_domains: search.blocked_domains.clone(),
                    include_sources: true,
                    ..OpenAiResponsesWebSearchConfig::default()
                };
                let bundle = openai_responses_web_search_tool_bundle(&provider_config)?;
                builder
                    .provider_params_patch
                    .add_openai_web_search(&provider_config);
                bundle
            }
            ProviderApiKind::AnthropicMessages => {
                anthropic_messages_web_search_tool_bundle(search)?
            }
            ref api_kind => {
                return Err(ToolError::UnsupportedCapability {
                    message: format!(
                        "web.search supports OpenAI Responses and Anthropic Messages, got {api_kind:?}"
                    ),
                });
            }
        }
        .ok_or_else(|| ToolError::InvalidRequest {
            message: "web.search was enabled but did not produce a provider tool".to_owned(),
        })?;
        builder.add_provider_tool_bundle(bundle);
    }

    if config.web.fetch {
        let provider_config = WebFetchToolConfig::enabled();
        let bundle = if env.target.api_kind == ProviderApiKind::AnthropicMessages {
            anthropic_messages_web_fetch_tool_bundle(&provider_config)?
        } else {
            web_fetch_tool_bundle(&provider_config)?
        }
        .ok_or_else(|| ToolError::InvalidRequest {
            message: "web.fetch was enabled but did not produce a tool".to_owned(),
        })?;
        if env.target.api_kind == ProviderApiKind::AnthropicMessages {
            builder.add_provider_tool_bundle(bundle);
        } else {
            builder.add_web_fetch(bundle);
        }
    }

    if config.environment_read || config.environment_selection {
        builder.add_environment_control(config.environment_selection)?;
    }

    let mut concurrency = config.concurrency.clone();
    if config.builtin.environment.jobs_enabled() {
        concurrency.enabled = true;
    }
    if concurrency.enabled_or_timer() {
        builder.add_concurrency(&concurrency)?;
    }

    Ok(builder.finish())
}

struct ToolsetBuilder {
    tools: BTreeMap<ToolName, ToolSpec>,
    catalog: ToolCatalog,
    documents_by_ref: BTreeMap<engine::BlobRef, ToolDocument>,
    visible_tools: Vec<ToolName>,
    seen_tools: BTreeSet<ToolName>,
    provider_params_patch: ProviderParamsPatch,
}

impl ToolsetBuilder {
    fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
            catalog: ToolCatalog::new(),
            documents_by_ref: BTreeMap::new(),
            visible_tools: Vec::new(),
            seen_tools: BTreeSet::new(),
            provider_params_patch: ProviderParamsPatch::default(),
        }
    }

    fn add_builtin_tools(
        &mut self,
        target: &ToolTarget,
        config: &BuiltinToolsetConfig,
    ) -> ToolResult<()> {
        let surface = config.presentation.surface(target);
        let omit_unsupported = config.presentation == BuiltinToolPresentation::ProviderDefault;
        for operation in config.vfs.operations() {
            let tool = BuiltinTool::vfs(operation, surface);
            let bundle = match tool.spec_bundle(target, STATIC_SCOPED_FS_PATHS) {
                Ok(bundle) => bundle,
                Err(ToolError::UnsupportedCapability { .. }) if omit_unsupported => continue,
                Err(error) => return Err(error),
            };
            let binding = tool.binding(target, config.dispatch.clone());
            self.add_bundle(bundle);
            self.catalog.insert(binding);
        }
        let one_shot = !config.environment.continue_process;
        for operation in config.environment.operations() {
            for tool in BuiltinTool::environment(operation, surface)
                .with_one_shot(one_shot)
                .variants()
            {
                let bundle = match tool.spec_bundle(target, STATIC_SCOPED_FS_PATHS) {
                    Ok(bundle) => bundle,
                    Err(ToolError::UnsupportedCapability { .. }) if omit_unsupported => continue,
                    Err(error) => return Err(error),
                };
                let binding = tool.binding(target, config.dispatch.clone());
                self.add_bundle(bundle);
                self.catalog.insert(binding);
            }
        }
        Ok(())
    }

    fn add_provider_tool_bundle(&mut self, bundle: ToolSpecBundle) {
        self.add_bundle(bundle);
    }

    fn add_web_fetch(&mut self, bundle: ToolSpecBundle) {
        self.add_bundle(bundle);
        self.catalog
            .insert(web_fetch_tool_binding(ToolDispatchMode::Local));
    }

    fn add_concurrency(&mut self, config: &ConcurrencyToolsetConfig) -> ToolResult<()> {
        for bundle in concurrency_tool_bundles(config)? {
            self.add_bundle(bundle);
        }
        for binding in concurrency_tool_bindings(ToolDispatchMode::Local, config) {
            self.catalog.insert(binding);
        }
        Ok(())
    }

    fn add_environment_control(&mut self, selection_tools: bool) -> ToolResult<()> {
        for bundle in environment_control_tool_bundles(selection_tools)? {
            self.add_bundle(bundle);
        }
        for binding in environment_control_tool_bindings(ToolDispatchMode::Local, selection_tools) {
            self.catalog.insert(binding);
        }
        Ok(())
    }

    fn add_bundle(&mut self, bundle: ToolSpecBundle) {
        let tool_name = bundle.spec.name.clone();
        if !self.seen_tools.insert(tool_name.clone()) {
            return;
        }
        for document in bundle.documents {
            self.documents_by_ref
                .entry(document.blob_ref.clone())
                .or_insert(document);
        }
        self.tools.insert(tool_name.clone(), bundle.spec);
        self.visible_tools.push(tool_name);
    }

    fn finish(self) -> ResolvedToolset {
        ResolvedToolset {
            tools: self.tools,
            documents: self.documents_by_ref.into_values().collect(),
            catalog: self.catalog,
            provider_params_patch: self.provider_params_patch,
        }
    }
}

const STATIC_SCOPED_FS_PATHS: bool = true;

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::concurrency::{AWAIT_TOOL_NAME, CANCEL_TOOL_NAME, SLEEP_TOOL_NAME};
    use crate::subagents::AGENT_SPAWN_TOOL_NAME;
    use crate::web::fetch::WEB_FETCH_TOOL_NAME;
    use crate::web::search::WebSearchToolConfig;

    fn target(api_kind: ProviderApiKind) -> ToolTarget {
        ToolTarget::api_kind(api_kind)
    }

    fn visible_names(toolset: &ResolvedToolset) -> Vec<String> {
        toolset
            .tools
            .keys()
            .map(|name| name.as_str().to_owned())
            .collect()
    }

    fn input_schema(toolset: &ResolvedToolset, name: &str) -> Value {
        let spec = toolset
            .tools
            .get(&ToolName::new(name))
            .unwrap_or_else(|| panic!("tool {name} is listed"));
        let engine::ToolKind::Function(function) = &spec.kind else {
            panic!("{name} is a function tool");
        };
        let document = toolset
            .documents
            .iter()
            .find(|document| document.blob_ref == function.input_schema_ref)
            .expect("schema document");
        serde_json::from_slice(&document.bytes).expect("schema json")
    }

    fn property_names(schema: &Value) -> Vec<String> {
        schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .cloned()
            .collect()
    }

    fn process_config() -> ToolsetConfig {
        let mut config = ToolsetConfig::empty();
        config.builtin.environment = EnvironmentToolsetConfig {
            run_process: true,
            continue_process: true,
            ..EnvironmentToolsetConfig::disabled()
        };
        config
    }

    #[test]
    fn provider_defaults_pick_codex_claude_and_canonical_process_surfaces() {
        let config = process_config();

        let responses = target(ProviderApiKind::OpenAiResponses);
        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &responses }, &config).expect("toolset");
        assert_eq!(visible_names(&toolset), vec!["exec_command", "write_stdin"]);
        let exec = input_schema(&toolset, "exec_command");
        assert_eq!(
            property_names(&exec),
            [
                "cmd",
                "login",
                "max_output_tokens",
                "tty",
                "workdir",
                "yield_time_ms"
            ]
        );
        assert_eq!(exec["required"], json!(["cmd"]));
        let write = input_schema(&toolset, "write_stdin");
        assert_eq!(
            property_names(&write),
            ["chars", "max_output_tokens", "session_id", "yield_time_ms"]
        );
        assert_eq!(
            toolset
                .catalog
                .get(&ToolName::new("write_stdin"))
                .expect("binding")
                .logical_id,
            "env.continue_process"
        );

        let anthropic = target(ProviderApiKind::AnthropicMessages);
        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &anthropic }, &config).expect("toolset");
        assert_eq!(
            visible_names(&toolset),
            vec!["Bash", "BashOutput", "KillShell"]
        );
        assert_eq!(
            property_names(&input_schema(&toolset, "Bash")),
            [
                "command",
                "dangerouslyDisableSandbox",
                "description",
                "run_in_background",
                "timeout"
            ]
        );
        assert_eq!(
            property_names(&input_schema(&toolset, "BashOutput")),
            ["bash_id", "filter", "timeout"]
        );
        assert_eq!(
            property_names(&input_schema(&toolset, "KillShell")),
            ["shell_id"]
        );
        for name in ["BashOutput", "KillShell"] {
            assert_eq!(
                toolset
                    .catalog
                    .get(&ToolName::new(name))
                    .expect("binding")
                    .logical_id,
                "env.continue_process"
            );
        }

        let completions = target(ProviderApiKind::OpenAiCompletions);
        let toolset = resolve_toolset(
            ToolsetEnvironment {
                target: &completions,
            },
            &config,
        )
        .expect("toolset");
        assert_eq!(
            visible_names(&toolset),
            vec!["continue_process", "run_process"]
        );
        let run = input_schema(&toolset, "run_process");
        assert_eq!(
            property_names(&run),
            [
                "argv",
                "cwd",
                "env",
                "max_output_bytes",
                "stdin",
                "timeout_ms",
                "tty",
                "yield_ms"
            ]
        );
        assert_eq!(
            property_names(&input_schema(&toolset, "continue_process")),
            [
                "close_stdin",
                "handle",
                "input",
                "max_output_bytes",
                "signal",
                "wait_ms"
            ]
        );
    }

    #[test]
    fn one_shot_policy_renders_the_restricted_shape_on_every_surface() {
        let mut config = process_config();
        config.builtin.environment.continue_process = false;

        let responses = target(ProviderApiKind::OpenAiResponses);
        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &responses }, &config).expect("toolset");
        assert_eq!(visible_names(&toolset), vec!["exec_command"]);
        assert_eq!(
            property_names(&input_schema(&toolset, "exec_command")),
            ["cmd", "login", "max_output_tokens", "timeout_ms", "workdir"]
        );
        assert_eq!(
            toolset
                .catalog
                .get(&ToolName::new("exec_command"))
                .expect("binding")
                .adapter_id
                .as_deref(),
            Some("codex-oneshot")
        );

        let anthropic = target(ProviderApiKind::AnthropicMessages);
        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &anthropic }, &config).expect("toolset");
        assert_eq!(visible_names(&toolset), vec!["Bash"]);
        assert!(
            !property_names(&input_schema(&toolset, "Bash"))
                .contains(&"run_in_background".to_owned())
        );

        let completions = target(ProviderApiKind::OpenAiCompletions);
        let toolset = resolve_toolset(
            ToolsetEnvironment {
                target: &completions,
            },
            &config,
        )
        .expect("toolset");
        assert_eq!(visible_names(&toolset), vec!["run_process"]);
        assert!(
            !property_names(&input_schema(&toolset, "run_process"))
                .contains(&"yield_ms".to_owned())
        );
    }

    #[test]
    fn workspace_toolset_renders_openai_canonical_builtin_tools() {
        let target = target(ProviderApiKind::OpenAiResponses);

        let toolset = resolve_toolset(
            ToolsetEnvironment { target: &target },
            &ToolsetConfig::workspace(),
        )
        .expect("toolset");

        assert_eq!(
            visible_names(&toolset),
            vec![
                "vfs_apply_patch",
                "vfs_edit_file",
                "vfs_glob",
                "vfs_grep",
                "vfs_list_dir",
                "vfs_read_file",
                "vfs_write_file"
            ]
        );
        assert!(
            toolset
                .catalog
                .get(&ToolName::new("vfs_read_file"))
                .is_some()
        );
        assert!(toolset.provider_params_patch.is_empty());
    }

    #[test]
    fn read_only_vfs_toolset_exposes_only_four_vfs_tools() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.builtin.vfs = FilesystemToolsetConfig::read_only();

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(
            visible_names(&toolset),
            vec!["vfs_glob", "vfs_grep", "vfs_list_dir", "vfs_read_file"]
        );
        assert!(
            toolset
                .catalog
                .bindings()
                .all(|binding| binding.logical_id.starts_with("vfs."))
        );
    }

    #[test]
    fn environment_and_vfs_toolsets_are_non_colliding() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::workspace();
        config.builtin.environment = EnvironmentToolsetConfig::basic();

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");
        let names = visible_names(&toolset);

        assert!(names.contains(&"vfs_read_file".to_owned()));
        assert!(names.contains(&"read_file".to_owned()));
        assert!(names.contains(&"exec_command".to_owned()));
        assert_eq!(names.len(), 16);
        assert!(
            toolset
                .catalog
                .bindings()
                .any(|binding| binding.logical_id == "vfs.read_file")
        );
        assert!(
            toolset
                .catalog
                .bindings()
                .any(|binding| binding.logical_id == "env.read_file")
        );
    }

    #[test]
    fn job_toolset_adds_suspension_tools_without_subagent_tools() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.builtin.environment = EnvironmentToolsetConfig::jobs();

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");
        let names = visible_names(&toolset);

        assert!(names.contains(&"job_submit".to_owned()));
        assert!(names.contains(&"job_read".to_owned()));
        assert!(names.contains(&AWAIT_TOOL_NAME.to_owned()));
        assert!(names.contains(&CANCEL_TOOL_NAME.to_owned()));
        assert!(!names.contains(&AGENT_SPAWN_TOOL_NAME.to_owned()));
        assert!(
            toolset
                .catalog
                .get(&ToolName::new(AWAIT_TOOL_NAME))
                .is_some()
        );
        assert!(
            toolset
                .catalog
                .get(&ToolName::new(CANCEL_TOOL_NAME))
                .is_some()
        );
    }

    #[test]
    fn environment_read_is_independent_from_default_off_selection_tools() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let disabled = resolve_toolset(
            ToolsetEnvironment { target: &target },
            &ToolsetConfig::empty(),
        )
        .expect("disabled toolset");
        assert!(
            visible_names(&disabled)
                .iter()
                .all(|name| !name.starts_with("environment_"))
        );

        let mut config = ToolsetConfig::empty();
        config.environment_read = true;
        let read_only = resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("environment read toolset");
        assert_eq!(visible_names(&read_only), vec!["environment_read"]);

        config.environment_selection = true;
        let enabled = resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("selection toolset");

        assert_eq!(
            visible_names(&enabled),
            vec![
                "environment_activate",
                "environment_deactivate",
                "environment_list",
                "environment_read",
            ]
        );
    }

    #[test]
    fn timer_toolset_adds_sleep_and_concurrency_tools() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.concurrency = ConcurrencyToolsetConfig::timer();

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");
        let names = visible_names(&toolset);

        assert!(names.contains(&AWAIT_TOOL_NAME.to_owned()));
        assert!(names.contains(&CANCEL_TOOL_NAME.to_owned()));
        assert!(names.contains(&SLEEP_TOOL_NAME.to_owned()));
        assert!(!names.contains(&AGENT_SPAWN_TOOL_NAME.to_owned()));
    }

    #[test]
    fn builtin_tool_presentation_defaults_to_claude_style_for_anthropic() {
        let target = target(ProviderApiKind::AnthropicMessages);
        let mut config = ToolsetConfig::empty();
        config.builtin = BuiltinToolsetConfig {
            vfs: FilesystemToolsetConfig {
                read_file: true,
                ..FilesystemToolsetConfig::disabled()
            },
            ..BuiltinToolsetConfig::disabled()
        };

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec!["VfsRead"]);
        assert!(
            toolset
                .documents
                .iter()
                .any(|document| document.text_lossy().contains("\"file_path\""))
        );
    }

    #[test]
    fn workspace_provider_default_omits_builtin_tools_unsupported_by_provider_surface() {
        let target = target(ProviderApiKind::AnthropicMessages);

        let toolset = resolve_toolset(
            ToolsetEnvironment { target: &target },
            &ToolsetConfig::workspace(),
        )
        .expect("toolset");

        assert_eq!(
            visible_names(&toolset),
            vec![
                "VfsEdit",
                "VfsGlob",
                "VfsGrep",
                "VfsListDir",
                "VfsRead",
                "VfsWrite"
            ]
        );
    }

    #[test]
    fn anthropic_list_dir_names_preserve_vfs_and_environment_domains() {
        let target = target(ProviderApiKind::AnthropicMessages);
        let mut config = ToolsetConfig::empty();
        config.builtin.vfs.list_dir = true;
        config.builtin.environment.filesystem.list_dir = true;

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec!["ListDir", "VfsListDir"]);
        assert_eq!(
            toolset
                .catalog
                .get(&ToolName::new("ListDir"))
                .expect("environment list binding")
                .logical_id,
            "env.list_dir"
        );
        assert_eq!(
            toolset
                .catalog
                .get(&ToolName::new("VfsListDir"))
                .expect("VFS list binding")
                .logical_id,
            "vfs.list_dir"
        );
    }

    #[test]
    fn web_search_adds_provider_native_tool_and_defaults_patch() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.web.search = Some(WebSearchToolConfig::new(
            vec!["docs.rs".to_owned()],
            Vec::new(),
        ));

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec!["web_search"]);
        assert!(toolset.catalog.is_empty());
        let native: Value =
            serde_json::from_slice(&toolset.documents[0].bytes).expect("native tool json");
        assert_eq!(
            native,
            json!({
                "type": "web_search",
                "external_web_access": false,
                "filters": { "allowed_domains": ["docs.rs"] }
            })
        );

        assert_eq!(
            toolset.provider_params_patch.openai_responses_include(),
            [crate::web::search::OPENAI_RESPONSES_WEB_SEARCH_SOURCES_INCLUDE.to_owned()]
        );
    }

    #[test]
    fn web_search_uses_anthropic_hosted_tool() {
        let target = target(ProviderApiKind::AnthropicMessages);
        let mut config = ToolsetConfig::empty();
        config.web.search = Some(WebSearchToolConfig::default());

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec!["web_search"]);
        assert!(toolset.catalog.is_empty());
        let native: Value =
            serde_json::from_slice(&toolset.documents[0].bytes).expect("native tool json");
        assert_eq!(native["type"], json!("web_search_20250305"));
    }

    #[test]
    fn web_fetch_adds_standard_function_tool_and_catalog_binding() {
        let target = target(ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.web.fetch = true;

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec![WEB_FETCH_TOOL_NAME]);
        assert!(
            toolset
                .catalog
                .get(&ToolName::new(WEB_FETCH_TOOL_NAME))
                .is_some()
        );
        let _spec = toolset
            .tools
            .get(&ToolName::new(WEB_FETCH_TOOL_NAME))
            .expect("web_fetch spec");
    }

    #[test]
    fn web_fetch_uses_anthropic_hosted_tool_without_local_binding() {
        let target = target(ProviderApiKind::AnthropicMessages);
        let mut config = ToolsetConfig::empty();
        config.web.fetch = true;

        let toolset =
            resolve_toolset(ToolsetEnvironment { target: &target }, &config).expect("toolset");

        assert_eq!(visible_names(&toolset), vec![WEB_FETCH_TOOL_NAME]);
        assert!(toolset.catalog.is_empty());
        let spec = toolset
            .tools
            .get(&ToolName::new(WEB_FETCH_TOOL_NAME))
            .expect("web_fetch spec");
        assert!(matches!(spec.kind, engine::ToolKind::ProviderNative(_)));
    }
}
