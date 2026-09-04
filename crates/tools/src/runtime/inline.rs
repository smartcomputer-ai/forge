//! Inline tool invocation runtime for CoreAgent tool calls.

use std::sync::Arc;

use async_trait::async_trait;
use engine::{
    CoreAgentIoError, CoreAgentTools, ToolBatchOutcome, ToolCallStatus, ToolInvocationBatchRequest,
    ToolInvocationBatchResult, ToolInvocationRequest, ToolInvocationResult, ToolName,
    storage::BlobStore,
};
use serde_json::Value;

use crate::{
    builtin::{BuiltinTool, BuiltinToolContext, BuiltinToolDomain},
    environment::EnvironmentToolContext,
    error::{ToolError, ToolResult},
    fs::FsToolContext,
    limits::ToolLimits,
    runtime::{ToolBinding, ToolCatalog, ToolDispatchMode, ToolInvocationOutput, ToolRuntime},
    web::fetch::{WEB_FETCH_LOGICAL_ID, invoke_web_fetch},
};

#[derive(Clone)]
pub struct InlineToolRuntime {
    vfs: Option<FsToolContext>,
    environment: Option<EnvironmentToolContext>,
    catalog: ToolCatalog,
    blobs: Arc<dyn BlobStore>,
    limits: ToolLimits,
}

impl InlineToolRuntime {
    pub fn with_vfs_filesystem(ctx: FsToolContext, catalog: ToolCatalog) -> Self {
        let blobs = ctx.blobs.clone();
        let limits = ctx.limits;
        Self::with_contexts_and_blob_store(Some(ctx), None, blobs, limits, catalog)
    }

    pub fn with_environment(ctx: EnvironmentToolContext, catalog: ToolCatalog) -> Self {
        let blobs = ctx.blobs.clone();
        let limits = ctx.limits;
        Self::with_contexts_and_blob_store(None, Some(ctx), blobs, limits, catalog)
    }

    pub fn with_contexts_and_blob_store(
        vfs: Option<FsToolContext>,
        environment: Option<EnvironmentToolContext>,
        blobs: Arc<dyn BlobStore>,
        limits: ToolLimits,
        catalog: ToolCatalog,
    ) -> Self {
        Self {
            vfs,
            environment,
            catalog,
            blobs,
            limits,
        }
    }

    pub fn vfs_context(&self) -> Option<&crate::fs::FsToolContext> {
        self.vfs.as_ref()
    }

    pub fn environment_context(&self) -> Option<&EnvironmentToolContext> {
        self.environment.as_ref()
    }

    pub fn catalog(&self) -> &ToolCatalog {
        &self.catalog
    }

    pub async fn invoke_call(
        &self,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let binding = match self.resolve_binding(call) {
            Ok(binding) => binding,
            Err(error) => return self.failed_result_without_context(call, error).await,
        };
        if binding.logical_id == WEB_FETCH_LOGICAL_ID {
            let arguments = match self.read_arguments_from_blobs(call).await {
                Ok(arguments) => arguments,
                Err(error) => return self.failed_result_without_context(call, error).await,
            };
            return match self
                .invoke_json_with_binding(None, &binding, &call.tool_name, arguments)
                .await
            {
                Ok(output) => self.succeeded_result_without_context(call, output).await,
                Err(error) => self.failed_result_without_context(call, error).await,
            };
        }

        let ctx = match self.resolve_call_context(&binding) {
            Ok(ctx) => ctx,
            Err(error) => return self.target_error_result(call, error).await,
        };
        ctx.drain_tool_effects();
        let arguments = match self.read_arguments(ctx, call).await {
            Ok(arguments) => arguments,
            Err(error) => return self.failed_result(ctx, call, error).await,
        };

        match self
            .invoke_json_with_binding(Some(ctx), &binding, &call.tool_name, arguments)
            .await
        {
            Ok(output) => self.succeeded_result(ctx, call, output).await,
            Err(error) => self.failed_result(ctx, call, error).await,
        }
    }

    fn resolve_binding(&self, call: &ToolInvocationRequest) -> ToolResult<ToolBinding> {
        self.catalog
            .get(&call.tool_name)
            .cloned()
            .ok_or_else(|| ToolError::UnsupportedCapability {
                message: format!("unknown tool: {}", call.tool_name),
            })
    }

    fn resolve_call_context(&self, binding: &ToolBinding) -> ToolResult<BuiltinToolContext<'_>> {
        let tool = BuiltinTool::from_binding(
            &binding.logical_id,
            binding.adapter_id.as_deref(),
            binding.tool_name.as_str(),
        )
        .ok_or_else(|| ToolError::UnsupportedCapability {
            message: format!("unsupported tool binding: {}", binding.logical_id),
        })?;
        match tool.domain() {
            BuiltinToolDomain::Vfs => {
                self.vfs
                    .as_ref()
                    .map(BuiltinToolContext::Vfs)
                    .ok_or_else(|| ToolError::InvalidRequest {
                        message: "no_vfs_workspace_links".to_owned(),
                    })
            }
            BuiltinToolDomain::Environment => {
                let ctx = self
                    .environment
                    .as_ref()
                    .ok_or_else(|| ToolError::InvalidRequest {
                        message: "no_active_environment".to_owned(),
                    })?;
                if tool.is_filesystem_operation()
                    && tool.requires_write()
                    && ctx
                        .filesystem
                        .as_ref()
                        .is_some_and(|filesystem| filesystem.fs.access_policy().is_read_only())
                {
                    return Err(ToolError::UnsupportedCapability {
                        message: "environment_filesystem_read_only".to_owned(),
                    });
                }
                Ok(BuiltinToolContext::Environment(ctx))
            }
        }
    }

    async fn read_arguments(
        &self,
        ctx: BuiltinToolContext<'_>,
        call: &ToolInvocationRequest,
    ) -> ToolResult<Value> {
        let bytes = ctx.blobs().read_bytes(&call.arguments_ref).await?;
        serde_json::from_slice(&bytes).map_err(|error| ToolError::InvalidRequest {
            message: format!("invalid JSON tool arguments: {error}"),
        })
    }

    async fn read_arguments_from_blobs(&self, call: &ToolInvocationRequest) -> ToolResult<Value> {
        let bytes = self.blobs.read_bytes(&call.arguments_ref).await?;
        serde_json::from_slice(&bytes).map_err(|error| ToolError::InvalidRequest {
            message: format!("invalid JSON tool arguments: {error}"),
        })
    }

    async fn succeeded_result(
        &self,
        ctx: BuiltinToolContext<'_>,
        call: &ToolInvocationRequest,
        output: ToolInvocationOutput,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let output_bytes = serde_json::to_vec(&output.output_json)
            .map_err(|error| io_error(format!("failed to encode tool output: {error}")))?;
        let output_ref = self.put_blob(ctx, output_bytes).await?;
        let visible = output.model_visible_text.into_bytes();
        let projection = projected(visible, ctx.limits().max_model_visible_output_bytes);
        let model_visible_ref = self.put_blob(ctx, projection.bytes).await?;

        let mut effects = output.effects;
        effects.extend(ctx.drain_tool_effects());

        Ok(ToolInvocationResult {
            duration_ms: None,
            output_bytes: Some(projection.output_bytes),
            truncated: projection.truncated,
            call_id: call.call_id.clone(),
            status: ToolCallStatus::Succeeded,
            output_ref: Some(output_ref),
            model_visible_context_entries: vec![ToolInvocationResult::tool_result_context_entry(
                &call.call_id,
                ToolCallStatus::Succeeded,
                model_visible_ref,
            )],
            error_ref: None,
            effects,
        })
    }

    async fn failed_result(
        &self,
        ctx: BuiltinToolContext<'_>,
        call: &ToolInvocationRequest,
        error: ToolError,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let projection = projected(
            format!("{error}").into_bytes(),
            ctx.limits().max_model_visible_output_bytes,
        );
        let error_ref = self.put_blob(ctx, projection.bytes).await?;

        Ok(ToolInvocationResult {
            duration_ms: None,
            output_bytes: Some(projection.output_bytes),
            truncated: projection.truncated,
            call_id: call.call_id.clone(),
            status: ToolCallStatus::Failed,
            output_ref: None,
            model_visible_context_entries: vec![ToolInvocationResult::tool_result_context_entry(
                &call.call_id,
                ToolCallStatus::Failed,
                error_ref.clone(),
            )],
            error_ref: Some(error_ref),
            effects: ctx.drain_tool_effects(),
        })
    }

    async fn succeeded_result_without_context(
        &self,
        call: &ToolInvocationRequest,
        output: ToolInvocationOutput,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let output_bytes = serde_json::to_vec(&output.output_json)
            .map_err(|error| io_error(format!("failed to encode tool output: {error}")))?;
        let output_ref = self.put_blob_bytes(output_bytes).await?;
        let projection = projected(
            output.model_visible_text.into_bytes(),
            self.limits.max_model_visible_output_bytes,
        );
        let model_visible_ref = self.put_blob_bytes(projection.bytes).await?;

        Ok(ToolInvocationResult {
            duration_ms: None,
            output_bytes: Some(projection.output_bytes),
            truncated: projection.truncated,
            call_id: call.call_id.clone(),
            status: ToolCallStatus::Succeeded,
            output_ref: Some(output_ref),
            model_visible_context_entries: vec![ToolInvocationResult::tool_result_context_entry(
                &call.call_id,
                ToolCallStatus::Succeeded,
                model_visible_ref,
            )],
            error_ref: None,
            effects: output.effects,
        })
    }

    async fn failed_result_without_context(
        &self,
        call: &ToolInvocationRequest,
        error: ToolError,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let projection = projected(
            error.to_string().into_bytes(),
            self.limits.max_model_visible_output_bytes,
        );
        let error_ref = self.put_blob_bytes(projection.bytes).await?;

        Ok(ToolInvocationResult {
            duration_ms: None,
            output_bytes: Some(projection.output_bytes),
            truncated: projection.truncated,
            call_id: call.call_id.clone(),
            status: ToolCallStatus::Failed,
            output_ref: None,
            model_visible_context_entries: vec![ToolInvocationResult::tool_result_context_entry(
                &call.call_id,
                ToolCallStatus::Failed,
                error_ref.clone(),
            )],
            error_ref: Some(error_ref),
            effects: Vec::new(),
        })
    }

    async fn target_error_result(
        &self,
        call: &ToolInvocationRequest,
        error: ToolError,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        self.failed_result_without_context(call, error).await
    }

    async fn put_blob(
        &self,
        ctx: BuiltinToolContext<'_>,
        bytes: Vec<u8>,
    ) -> Result<engine::BlobRef, CoreAgentIoError> {
        ctx.blobs()
            .put_bytes(bytes)
            .await
            .map_err(|error| io_error(format!("failed to write tool blob: {error}")))
    }

    async fn put_blob_bytes(&self, bytes: Vec<u8>) -> Result<engine::BlobRef, CoreAgentIoError> {
        self.blobs
            .put_bytes(bytes)
            .await
            .map_err(|error| io_error(format!("failed to write tool blob: {error}")))
    }

    async fn invoke_json_with_binding(
        &self,
        ctx: Option<BuiltinToolContext<'_>>,
        binding: &ToolBinding,
        tool_name: &ToolName,
        arguments: Value,
    ) -> ToolResult<ToolInvocationOutput> {
        if binding.dispatch != ToolDispatchMode::Local {
            return Err(ToolError::UnsupportedCapability {
                message: format!("tool {tool_name} is not configured for local dispatch"),
            });
        }
        if binding.logical_id == WEB_FETCH_LOGICAL_ID {
            return invoke_web_fetch(arguments).await;
        }
        let builtin_tool = BuiltinTool::from_binding(
            &binding.logical_id,
            binding.adapter_id.as_deref(),
            binding.tool_name.as_str(),
        )
        .ok_or_else(|| ToolError::UnsupportedCapability {
            message: format!("unsupported tool binding: {}", binding.logical_id),
        })?;
        let ctx = ctx.ok_or_else(|| ToolError::InvalidRequest {
            message: format!("tool {tool_name} requires its filesystem domain context"),
        })?;
        builtin_tool.invoke_json(ctx, arguments).await
    }
}

#[async_trait]
impl ToolRuntime for InlineToolRuntime {
    async fn invoke_json(
        &self,
        tool_name: &ToolName,
        arguments: Value,
    ) -> ToolResult<ToolInvocationOutput> {
        let binding =
            self.catalog
                .get(tool_name)
                .ok_or_else(|| ToolError::UnsupportedCapability {
                    message: format!("unknown tool: {tool_name}"),
                })?;
        if binding.logical_id == WEB_FETCH_LOGICAL_ID {
            return self
                .invoke_json_with_binding(None, binding, tool_name, arguments)
                .await;
        }
        let ctx = self.resolve_call_context(binding)?;
        ctx.drain_tool_effects();
        let mut output = self
            .invoke_json_with_binding(Some(ctx), binding, tool_name, arguments)
            .await?;
        output.effects.extend(ctx.drain_tool_effects());
        Ok(output)
    }
}

#[async_trait]
impl CoreAgentTools for InlineToolRuntime {
    async fn invoke_batch(
        &self,
        request: ToolInvocationBatchRequest,
    ) -> Result<ToolBatchOutcome, CoreAgentIoError> {
        let mut results = Vec::with_capacity(request.calls.len());
        for call in request.calls {
            results.push(self.invoke_call(&call).await?);
        }
        Ok(ToolBatchOutcome::completed(ToolInvocationBatchResult {
            run_id: request.run_id,
            turn_id: request.turn_id,
            batch_id: request.batch_id,
            results,
        }))
    }
}

/// Model-visible text after the projection budget, with the accounting
/// facts the completion event carries: how much the tool produced and
/// whether the budget cut it.
struct ProjectedText {
    bytes: Vec<u8>,
    output_bytes: u64,
    truncated: bool,
}

fn projected(bytes: Vec<u8>, max_bytes: u64) -> ProjectedText {
    let output_bytes = bytes.len() as u64;
    let truncated = output_bytes > max_bytes;
    ProjectedText {
        bytes: truncate_bytes(bytes, max_bytes),
        output_bytes,
        truncated,
    }
}

fn truncate_bytes(mut bytes: Vec<u8>, max_bytes: u64) -> Vec<u8> {
    let max_bytes = max_bytes as usize;
    if bytes.len() <= max_bytes {
        return bytes;
    }
    bytes.truncate(max_bytes);
    while std::str::from_utf8(&bytes).is_err() {
        bytes.pop();
    }
    bytes.extend_from_slice(b"\n[truncated]");
    bytes
}

fn io_error(message: impl Into<String>) -> CoreAgentIoError {
    CoreAgentIoError::Failed {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use engine::{
        BlobRef, ContextEntryKind, RunId, SessionId, ToolBatchId, ToolCallId,
        ToolInvocationRequest, ToolInvocationResult, ToolName, TurnId,
        storage::{BlobStore, InMemoryBlobStore},
    };
    use serde_json::json;

    use super::*;
    use crate::builtin::BuiltinToolOperation;
    use crate::environment::EnvironmentToolContext;
    use crate::environment::process::{
        ContinueProcessRequest, ProcessError, ProcessExecResult, ProcessExecutor, ProcessHandle,
        ProcessOutput, ProcessRequest, ProcessSignal, ProcessStatus, StreamOutput,
    };
    use crate::fs::{FileSystem, FsPath, InMemoryFileSystem};
    use crate::runtime::{ToolCatalog, ToolTarget};
    use crate::toolset::{
        BuiltinToolPresentation, BuiltinToolsetConfig, FilesystemToolsetConfig, ToolsetConfig,
        ToolsetEnvironment, resolve_toolset,
    };

    #[derive(Default)]
    struct RecordingProcessExecutor {
        requests: Mutex<Vec<ProcessRequest>>,
        continues: Mutex<Vec<ContinueProcessRequest>>,
    }

    #[async_trait]
    impl ProcessExecutor for RecordingProcessExecutor {
        async fn run_process(&self, request: ProcessRequest) -> ProcessExecResult<ProcessOutput> {
            let running = request.yield_ms == Some(0);
            self.requests.lock().expect("lock").push(request);
            Ok(ProcessOutput {
                status: if running {
                    ProcessStatus::Running
                } else {
                    ProcessStatus::Succeeded
                },
                handle: running.then(|| ProcessHandle::new("proc-1")),
                pid: Some(42),
                exit_code: (!running).then_some(0),
                failure: None,
                stdout: StreamOutput {
                    bytes: b"ok".to_vec(),
                    omitted_at: None,
                },
                stderr: StreamOutput::default(),
                omitted_bytes: 0,
                leftover_processes: Vec::new(),
            })
        }

        async fn continue_process(
            &self,
            request: ContinueProcessRequest,
        ) -> ProcessExecResult<ProcessOutput> {
            let killed = request.signal == Some(ProcessSignal::Kill);
            self.continues.lock().expect("lock").push(request);
            Ok(ProcessOutput {
                status: if killed {
                    ProcessStatus::Killed
                } else {
                    ProcessStatus::Succeeded
                },
                handle: None,
                pid: Some(42),
                exit_code: (!killed).then_some(0),
                failure: None,
                stdout: StreamOutput {
                    bytes: b"more".to_vec(),
                    omitted_at: None,
                },
                stderr: StreamOutput::default(),
                omitted_bytes: 0,
                leftover_processes: Vec::new(),
            })
        }
    }

    #[allow(dead_code)]
    fn unused(_: ProcessError) {}

    fn call(arguments_ref: BlobRef, tool_name: &str) -> ToolInvocationRequest {
        ToolInvocationRequest {
            call_id: ToolCallId::new("call-1"),
            tool_name: ToolName::new(tool_name),
            arguments_ref,
            workflow_tool: None,
            promise_control: None,
            remote_mcp: None,
        }
    }

    fn batch_request(call: ToolInvocationRequest) -> ToolInvocationBatchRequest {
        ToolInvocationBatchRequest {
            session_id: SessionId::new("session-a"),
            run_id: RunId::new(1),
            turn_id: TurnId::new(1),
            batch_id: ToolBatchId::new(1),
            promise_id_base: 1,
            active_environment_id: None,
            environment_policy: None,
            subagents_policy: None,
            workspace_links: Vec::new(),
            calls: vec![call],
        }
    }

    fn workspace_catalog(api_kind: engine::ProviderApiKind) -> ToolCatalog {
        let target = ToolTarget::api_kind(api_kind);
        resolve_toolset(
            ToolsetEnvironment { target: &target },
            &ToolsetConfig::workspace(),
        )
        .expect("toolset")
        .catalog
    }

    fn catalog_for_operations_with_presentation(
        api_kind: engine::ProviderApiKind,
        presentation: BuiltinToolPresentation,
        operations: impl IntoIterator<Item = BuiltinToolOperation>,
    ) -> ToolCatalog {
        let target = ToolTarget::api_kind(api_kind);
        let mut config = ToolsetConfig::empty();
        config.builtin = crate::toolset::BuiltinToolsetConfig::from_operations(operations);
        config.builtin.presentation = presentation;
        resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("toolset")
            .catalog
    }

    fn fs_context(fs: impl FileSystem + 'static, blobs: Arc<dyn BlobStore>) -> FsToolContext {
        FsToolContext::new(Arc::new(fs), blobs)
    }

    fn runtime_with_vfs(
        fs: impl FileSystem + 'static,
        blobs: Arc<dyn BlobStore>,
        catalog: ToolCatalog,
    ) -> InlineToolRuntime {
        InlineToolRuntime::with_vfs_filesystem(fs_context(fs, blobs), catalog)
    }

    fn web_fetch_catalog() -> ToolCatalog {
        let target = ToolTarget::api_kind(engine::ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::empty();
        config.web.fetch = true;
        resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("toolset")
            .catalog
    }

    fn visible_tool_result_ref(result: &ToolInvocationResult) -> BlobRef {
        result
            .model_visible_context_entries
            .iter()
            .find_map(|entry| {
                matches!(entry.kind, ContextEntryKind::ToolResult { .. })
                    .then(|| entry.content_ref.clone())
            })
            .expect("visible ref")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inline_runtime_maps_tool_name_to_builtin_operation() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(&FsPath::new("/file.txt").expect("path"), b"hello".to_vec())
            .await
            .expect("write file");
        let catalog = workspace_catalog(engine::ProviderApiKind::OpenAiResponses);
        let runtime = runtime_with_vfs(fs, blobs.clone(), catalog);

        let output = runtime
            .invoke_json(
                &ToolName::new("vfs_read_file"),
                json!({ "path": "/file.txt", "offset": null, "limit": null }),
            )
            .await
            .expect("invoke tool");

        assert!(output.model_visible_text.contains("hello"));
        assert_eq!(output.output_json["text"], "hello");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inline_runtime_maps_claude_code_like_tool_arguments() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(&FsPath::new("/file.txt").expect("path"), b"hello".to_vec())
            .await
            .expect("write file");
        let target = ToolTarget::api_kind(engine::ProviderApiKind::AnthropicMessages);
        let mut config = ToolsetConfig::empty();
        config.builtin = BuiltinToolsetConfig {
            presentation: BuiltinToolPresentation::ClaudeCodeLike,
            vfs: FilesystemToolsetConfig {
                read_file: true,
                ..FilesystemToolsetConfig::disabled()
            },
            ..BuiltinToolsetConfig::disabled()
        };
        let catalog = resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("toolset")
            .catalog;
        let runtime = runtime_with_vfs(fs, blobs, catalog);

        let output = runtime
            .invoke_json(
                &ToolName::new("VfsRead"),
                json!({ "file_path": "/file.txt", "offset": null, "limit": null }),
            )
            .await
            .expect("invoke tool");

        assert!(output.model_visible_text.contains("hello"));
        assert_eq!(output.output_json["text"], "hello");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn vfs_and_environment_file_tools_are_isolated() {
        let blobs: Arc<dyn BlobStore> = Arc::new(InMemoryBlobStore::new());
        let vfs = InMemoryFileSystem::full_access();
        let environment = InMemoryFileSystem::full_access();
        let path = FsPath::new("/same.txt").expect("path");
        vfs.write_file(&path, b"vfs".to_vec())
            .await
            .expect("vfs write");
        environment
            .write_file(&path, b"environment".to_vec())
            .await
            .expect("environment write");

        let target = ToolTarget::api_kind(engine::ProviderApiKind::OpenAiResponses);
        let mut config = ToolsetConfig::workspace();
        config.builtin.environment = crate::toolset::EnvironmentToolsetConfig::basic();
        let catalog = resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .expect("toolset")
            .catalog;
        let environment = EnvironmentToolContext::new(None, blobs.clone())
            .with_environment_id("environment-a")
            .with_filesystem(fs_context(environment, blobs.clone()));
        let runtime = InlineToolRuntime::with_contexts_and_blob_store(
            Some(fs_context(vfs, blobs.clone())),
            Some(environment),
            blobs,
            ToolLimits::default(),
            catalog,
        );

        let vfs_output = runtime
            .invoke_json(
                &ToolName::new("vfs_read_file"),
                json!({ "path": "/same.txt", "offset": null, "limit": null }),
            )
            .await
            .expect("read VFS");
        let environment_output = runtime
            .invoke_json(
                &ToolName::new("read_file"),
                json!({ "path": "/same.txt", "offset": null, "limit": null }),
            )
            .await
            .expect("read environment");

        assert_eq!(vfs_output.output_json["text"], "vfs");
        assert_eq!(environment_output.output_json["text"], "environment");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn environment_filesystem_failures_distinguish_missing_and_read_only() {
        let blobs: Arc<dyn BlobStore> = Arc::new(InMemoryBlobStore::new());
        let catalog = catalog_for_operations_with_presentation(
            engine::ProviderApiKind::OpenAiResponses,
            BuiltinToolPresentation::Canonical,
            [BuiltinToolOperation::WriteFile],
        );
        let arguments = json!({ "path": "/file.txt", "content": "new" });

        let unavailable = InlineToolRuntime::with_environment(
            EnvironmentToolContext::new(None, blobs.clone()).with_environment_id("environment-a"),
            catalog.clone(),
        )
        .invoke_json(&ToolName::new("write_file"), arguments.clone())
        .await
        .expect_err("missing filesystem");
        assert!(
            unavailable
                .to_string()
                .contains("environment_filesystem_unavailable")
        );

        let read_only = InMemoryFileSystem::new(crate::fs::FileAccessPolicy::FullReadOnly);
        let read_only = InlineToolRuntime::with_environment(
            EnvironmentToolContext::new(None, blobs.clone())
                .with_environment_id("environment-a")
                .with_filesystem(fs_context(read_only, blobs)),
            catalog,
        )
        .invoke_json(&ToolName::new("write_file"), arguments)
        .await
        .expect_err("read-only filesystem");
        assert!(
            read_only
                .to_string()
                .contains("environment_filesystem_read_only")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn core_tools_reads_arguments_and_writes_result_blobs() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(&FsPath::new("/file.txt").expect("path"), b"hello".to_vec())
            .await
            .expect("write file");
        let catalog = workspace_catalog(engine::ProviderApiKind::OpenAiResponses);
        let runtime = runtime_with_vfs(fs, blobs.clone(), catalog);
        let args_ref = blobs
            .put_bytes(br#"{"path":"/file.txt","offset":null,"limit":null}"#.to_vec())
            .await
            .expect("write args");

        let result = runtime
            .invoke_batch(batch_request(call(args_ref, "vfs_read_file")))
            .await
            .expect("invoke batch")
            .completed_result()
            .expect("completed batch")
            .single_result()
            .expect("single result");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        let visible_ref = visible_tool_result_ref(&result);
        let visible = blobs.read_text(&visible_ref).await.expect("visible text");
        assert!(visible.contains("hello"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn core_tools_invokes_batch_and_writes_result_blobs() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = InMemoryFileSystem::full_access();
        fs.write_file(&FsPath::new("/file.txt").expect("path"), b"hello".to_vec())
            .await
            .expect("write file");
        let catalog = workspace_catalog(engine::ProviderApiKind::OpenAiResponses);
        let runtime = runtime_with_vfs(fs, blobs.clone(), catalog);
        let args_ref = blobs
            .put_bytes(br#"{"path":"/file.txt","offset":null,"limit":null}"#.to_vec())
            .await
            .expect("write args");

        let result = CoreAgentTools::invoke_batch(
            &runtime,
            ToolInvocationBatchRequest {
                session_id: SessionId::new("session-a"),
                run_id: RunId::new(1),
                turn_id: TurnId::new(1),
                batch_id: ToolBatchId::new(1),
                promise_id_base: 1,
                active_environment_id: None,
                environment_policy: None,
                subagents_policy: None,
                workspace_links: Vec::new(),
                calls: vec![engine::ToolInvocationRequest {
                    call_id: ToolCallId::new("call-1"),
                    tool_name: ToolName::new("vfs_read_file"),
                    arguments_ref: args_ref,
                    workflow_tool: None,
                    promise_control: None,
                    remote_mcp: None,
                }],
            },
        )
        .await
        .expect("invoke batch")
        .completed_result()
        .expect("completed batch");

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].status, ToolCallStatus::Succeeded);
        let visible_ref = visible_tool_result_ref(&result.results[0]);
        let visible = blobs.read_text(&visible_ref).await.expect("visible text");
        assert!(visible.contains("hello"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn web_fetch_does_not_require_a_filesystem_domain() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let runtime = InlineToolRuntime::with_contexts_and_blob_store(
            None,
            None,
            blobs.clone(),
            ToolLimits::default(),
            web_fetch_catalog(),
        );
        let args_ref = blobs
            .put_bytes(br#"{"url":"http://127.0.0.1:1/","max_chars":1000}"#.to_vec())
            .await
            .expect("write args");

        let result = runtime
            .invoke_batch(batch_request(call(args_ref, "web_fetch")))
            .await
            .expect("invoke batch")
            .completed_result()
            .expect("completed batch")
            .single_result()
            .expect("single result");

        assert_eq!(result.status, ToolCallStatus::Failed);
        let error_ref = result.error_ref.expect("error ref");
        let error = blobs.read_text(&error_ref).await.expect("error text");
        assert!(error.contains("non-public"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn core_tools_routes_process_tools_to_active_environment() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let process = Arc::new(RecordingProcessExecutor::default());
        let process_ctx: Arc<dyn ProcessExecutor> = process.clone();
        let env_ctx = EnvironmentToolContext::new(Some(process_ctx), blobs.clone());
        let catalog = catalog_for_operations_with_presentation(
            engine::ProviderApiKind::OpenAiResponses,
            BuiltinToolPresentation::Canonical,
            [BuiltinToolOperation::RunProcess],
        );
        let runtime = InlineToolRuntime::with_environment(env_ctx, catalog);
        let args_ref = blobs
            .put_bytes(br#"{"argv":["echo","hello"]}"#.to_vec())
            .await
            .expect("write args");

        let result = runtime
            .invoke_batch(batch_request(call(args_ref, "run_process")))
            .await
            .expect("invoke batch")
            .completed_result()
            .expect("completed batch")
            .single_result()
            .expect("single result");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        let visible_ref = visible_tool_result_ref(&result);
        let visible = blobs.read_text(&visible_ref).await.expect("visible text");
        assert!(visible.contains("ok"));
        assert!(visible.contains("[exited with code 0]"));
        let requests = process.requests.lock().expect("lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].argv, ["echo", "hello"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn claude_code_like_process_tools_dispatch_by_variant() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let process = Arc::new(RecordingProcessExecutor::default());
        let process_ctx: Arc<dyn ProcessExecutor> = process.clone();
        let env_ctx = EnvironmentToolContext::new(Some(process_ctx), blobs.clone());
        let catalog = catalog_for_operations_with_presentation(
            engine::ProviderApiKind::AnthropicMessages,
            BuiltinToolPresentation::ProviderDefault,
            [
                BuiltinToolOperation::RunProcess,
                BuiltinToolOperation::ContinueProcess,
            ],
        );
        let runtime = InlineToolRuntime::with_environment(env_ctx, catalog);

        let started = runtime
            .invoke_json(
                &ToolName::new("Bash"),
                json!({ "command": "sleep 5", "run_in_background": true }),
            )
            .await
            .expect("Bash");
        assert!(
            started
                .model_visible_text
                .contains("Command running in background with ID: proc-1")
        );
        assert!(started.model_visible_text.contains("BashOutput"));
        assert!(started.model_visible_text.contains("KillShell"));

        let read = runtime
            .invoke_json(&ToolName::new("BashOutput"), json!({ "bash_id": "proc-1" }))
            .await
            .expect("BashOutput");
        assert!(read.model_visible_text.contains("[exited with code 0]"));

        let killed = runtime
            .invoke_json(&ToolName::new("KillShell"), json!({ "shell_id": "proc-1" }))
            .await
            .expect("KillShell");
        assert!(killed.model_visible_text.ends_with("[killed]"));

        let requests = process.requests.lock().expect("lock");
        assert_eq!(requests[0].yield_ms, Some(0));
        assert_eq!(requests[0].timeout_ms, None);
        let continues = process.continues.lock().expect("lock");
        assert_eq!(continues.len(), 2);
        assert_eq!(continues[0].signal, None);
        assert_eq!(continues[1].signal, Some(ProcessSignal::Kill));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn codex_like_exec_command_is_the_openai_responses_default() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let process = Arc::new(RecordingProcessExecutor::default());
        let process_ctx: Arc<dyn ProcessExecutor> = process.clone();
        let env_ctx = EnvironmentToolContext::new(Some(process_ctx), blobs.clone());
        let catalog = catalog_for_operations_with_presentation(
            engine::ProviderApiKind::OpenAiResponses,
            BuiltinToolPresentation::ProviderDefault,
            [
                BuiltinToolOperation::RunProcess,
                BuiltinToolOperation::ContinueProcess,
            ],
        );
        let runtime = InlineToolRuntime::with_environment(env_ctx, catalog);

        let output = runtime
            .invoke_json(
                &ToolName::new("exec_command"),
                json!({ "cmd": "echo hi", "yield_time_ms": 250 }),
            )
            .await
            .expect("exec_command");
        assert!(output.model_visible_text.starts_with("Wall time: "));
        assert!(
            output
                .model_visible_text
                .contains("Process exited with code 0\nOutput:\nok")
        );
        {
            let requests = process.requests.lock().expect("lock");
            assert_eq!(requests[0].argv, ["bash", "-lc", "echo hi"]);
            assert_eq!(requests[0].yield_ms, Some(250));
            assert_eq!(requests[0].timeout_ms, None);
        }

        let polled = runtime
            .invoke_json(
                &ToolName::new("write_stdin"),
                json!({ "session_id": "proc-1", "chars": "" }),
            )
            .await
            .expect("write_stdin");
        assert!(
            polled
                .model_visible_text
                .contains("Process exited with code 0")
        );
        let continues = process.continues.lock().expect("lock");
        assert_eq!(continues[0].input, None);
        assert_eq!(continues[0].wait_ms, Some(60_000));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn core_tools_fail_process_tools_without_active_environment() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let catalog = catalog_for_operations_with_presentation(
            engine::ProviderApiKind::OpenAiResponses,
            BuiltinToolPresentation::Canonical,
            [BuiltinToolOperation::RunProcess],
        );
        let runtime = InlineToolRuntime::with_contexts_and_blob_store(
            None,
            None,
            blobs.clone(),
            ToolLimits::default(),
            catalog,
        );
        let args_ref = blobs
            .put_bytes(br#"{"argv":["echo","hello"]}"#.to_vec())
            .await
            .expect("write args");

        let result = runtime
            .invoke_batch(batch_request(call(args_ref, "run_process")))
            .await
            .expect("invoke batch")
            .completed_result()
            .expect("completed batch")
            .single_result()
            .expect("single result");

        assert_eq!(result.status, ToolCallStatus::Failed);
        let error_ref = result.error_ref.expect("error ref");
        let error = blobs.read_text(&error_ref).await.expect("error text");
        assert!(error.contains("no_active_environment"));
    }
}
