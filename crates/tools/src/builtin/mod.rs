//! Built-in filesystem and environment action tool definitions.

use engine::{
    FunctionToolSpec, ToolExecutionClass, ToolExecutionSpec, ToolKind, ToolName, ToolParallelism,
    ToolSpec,
};
use serde_json::Value;

use crate::{
    environment::EnvironmentToolContext,
    error::{ToolError, ToolResult},
    fs::FsToolContext,
    runtime::{
        ToolBinding, ToolDispatchMode, ToolDocument, ToolInvocationOutput, ToolSpecBundle,
        ToolTarget,
    },
};

mod canonical;
mod claude;
mod codex;
mod shared;

pub use crate::environment::tools::{
    RunProcessArgs, WriteProcessStdinArgs, invoke_job_read, invoke_job_submit, invoke_run_process,
    invoke_write_process_stdin,
};
pub use crate::fs::tools::{
    ApplyPatchArgs, ApplyPatchResult, EditFileArgs, EditFileResult, GlobArgs, GlobResult, GrepArgs,
    GrepMatch, GrepResult, ListDirArgs, ListDirEntry, ReadFileArgs, ReadFileResult, WriteFileArgs,
    WriteFileResult, invoke_apply_patch, invoke_edit_file, invoke_glob, invoke_grep,
    invoke_list_dir, invoke_read_file, invoke_write_file,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BuiltinToolOperation {
    ReadFile,
    WriteFile,
    EditFile,
    ApplyPatch,
    Grep,
    Glob,
    ListDir,
    RunProcess,
    WriteProcessStdin,
    JobSubmit,
    JobRun,
    JobRead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BuiltinToolSurface {
    Canonical,
    CodexLike,
    ClaudeCodeLike,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BuiltinToolDomain {
    Vfs,
    Environment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BuiltinTool {
    domain: BuiltinToolDomain,
    operation: BuiltinToolOperation,
    surface: BuiltinToolSurface,
}

#[derive(Clone, Copy)]
pub enum BuiltinToolContext<'a> {
    Vfs(&'a FsToolContext),
    Environment(&'a EnvironmentToolContext),
}

impl<'a> BuiltinToolContext<'a> {
    pub fn filesystem(self) -> ToolResult<&'a FsToolContext> {
        match self {
            Self::Vfs(ctx) => Ok(ctx),
            Self::Environment(ctx) => {
                ctx.filesystem
                    .as_ref()
                    .ok_or_else(|| ToolError::UnsupportedCapability {
                        message: "environment_filesystem_unavailable".to_owned(),
                    })
            }
        }
    }

    pub fn environment(self) -> ToolResult<&'a EnvironmentToolContext> {
        match self {
            Self::Environment(ctx) => Ok(ctx),
            Self::Vfs(_) => Err(ToolError::InvalidRequest {
                message: "environment tool cannot use a VFS context".to_owned(),
            }),
        }
    }

    pub fn blobs(self) -> &'a std::sync::Arc<dyn engine::storage::BlobStore> {
        match self {
            Self::Vfs(ctx) => &ctx.blobs,
            Self::Environment(ctx) => &ctx.blobs,
        }
    }

    pub fn limits(self) -> crate::limits::ToolLimits {
        match self {
            Self::Vfs(ctx) => ctx.limits,
            Self::Environment(ctx) => ctx.limits,
        }
    }

    pub fn drain_tool_effects(self) -> Vec<engine::ToolEffect> {
        match self {
            Self::Vfs(ctx) => ctx.fs.drain_tool_effects(),
            Self::Environment(ctx) => ctx
                .filesystem
                .as_ref()
                .map_or_else(Vec::new, |ctx| ctx.fs.drain_tool_effects()),
        }
    }
}

impl BuiltinTool {
    pub const fn environment(operation: BuiltinToolOperation, surface: BuiltinToolSurface) -> Self {
        Self {
            domain: BuiltinToolDomain::Environment,
            operation,
            surface,
        }
    }

    pub const fn environment_canonical(operation: BuiltinToolOperation) -> Self {
        Self::environment(operation, BuiltinToolSurface::Canonical)
    }

    pub const fn vfs(operation: BuiltinToolOperation, surface: BuiltinToolSurface) -> Self {
        assert!(matches!(
            operation,
            BuiltinToolOperation::ReadFile
                | BuiltinToolOperation::WriteFile
                | BuiltinToolOperation::EditFile
                | BuiltinToolOperation::ApplyPatch
                | BuiltinToolOperation::Grep
                | BuiltinToolOperation::Glob
                | BuiltinToolOperation::ListDir
        ));
        Self {
            domain: BuiltinToolDomain::Vfs,
            operation,
            surface,
        }
    }

    pub const fn operation(self) -> BuiltinToolOperation {
        self.operation
    }

    pub const fn surface(self) -> BuiltinToolSurface {
        self.surface
    }

    pub const fn domain(self) -> BuiltinToolDomain {
        self.domain
    }

    pub const fn logical_id(self) -> &'static str {
        match (self.domain, self.operation) {
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::ReadFile) => "vfs.read_file",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::WriteFile) => "vfs.write_file",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::EditFile) => "vfs.edit_file",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::ApplyPatch) => "vfs.apply_patch",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::Grep) => "vfs.grep",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::Glob) => "vfs.glob",
            (BuiltinToolDomain::Vfs, BuiltinToolOperation::ListDir) => "vfs.list_dir",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::ReadFile) => "env.read_file",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::WriteFile) => "env.write_file",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::EditFile) => "env.edit_file",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::ApplyPatch) => "env.apply_patch",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::Grep) => "env.grep",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::Glob) => "env.glob",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::ListDir) => "env.list_dir",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::RunProcess) => "env.run_process",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::WriteProcessStdin) => {
                "env.write_process_stdin"
            }
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobSubmit) => "env.job_submit",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobRun) => "env.job_run",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobRead) => "env.job_read",
            (
                BuiltinToolDomain::Vfs,
                BuiltinToolOperation::RunProcess
                | BuiltinToolOperation::WriteProcessStdin
                | BuiltinToolOperation::JobSubmit
                | BuiltinToolOperation::JobRun
                | BuiltinToolOperation::JobRead,
            ) => unreachable!(),
        }
    }

    pub fn name_str(self) -> &'static str {
        if self.domain == BuiltinToolDomain::Vfs {
            return match (self.surface, self.operation) {
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::ReadFile,
                ) => "vfs_read_file",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::WriteFile,
                ) => "vfs_write_file",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::EditFile,
                ) => "vfs_edit_file",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::ApplyPatch,
                ) => "vfs_apply_patch",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::Grep,
                ) => "vfs_grep",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::Glob,
                ) => "vfs_glob",
                (
                    BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                    BuiltinToolOperation::ListDir,
                ) => "vfs_list_dir",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ReadFile) => "VfsRead",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::WriteFile) => "VfsWrite",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::EditFile) => "VfsEdit",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ApplyPatch) => {
                    "VfsApplyPatch"
                }
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Grep) => "VfsGrep",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Glob) => "VfsGlob",
                (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ListDir) => "VfsListDir",
                (
                    _,
                    BuiltinToolOperation::RunProcess
                    | BuiltinToolOperation::WriteProcessStdin
                    | BuiltinToolOperation::JobSubmit
                    | BuiltinToolOperation::JobRun
                    | BuiltinToolOperation::JobRead,
                ) => unreachable!(),
            };
        }
        match (self.surface, self.operation) {
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ReadFile,
            ) => "read_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::WriteFile,
            ) => "write_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::EditFile,
            ) => "edit_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ApplyPatch,
            ) => "apply_patch",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::Grep,
            ) => "grep",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::Glob,
            ) => "glob",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ListDir,
            ) => "list_dir",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::RunProcess,
            ) => "exec_command",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::WriteProcessStdin,
            ) => "write_stdin",
            (
                BuiltinToolSurface::Canonical
                | BuiltinToolSurface::CodexLike
                | BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::JobSubmit,
            ) => crate::environment::jobs::JOB_SUBMIT_TOOL_NAME,
            (
                BuiltinToolSurface::Canonical
                | BuiltinToolSurface::CodexLike
                | BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::JobRun,
            ) => crate::environment::jobs::JOB_RUN_TOOL_NAME,
            (
                BuiltinToolSurface::Canonical
                | BuiltinToolSurface::CodexLike
                | BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::JobRead,
            ) => crate::environment::jobs::JOB_READ_TOOL_NAME,
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ReadFile) => "Read",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::WriteFile) => "Write",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::EditFile) => "Edit",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Grep) => "Grep",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Glob) => "Glob",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::RunProcess) => "Bash",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ApplyPatch) => "apply_patch",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ListDir) => "ListDir",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::WriteProcessStdin) => {
                "write_stdin"
            }
        }
    }

    pub fn name(self, _target: &ToolTarget) -> ToolName {
        ToolName::new(self.name_str())
    }

    pub fn from_logical_id(logical_id: &str) -> Option<Self> {
        Some(match logical_id {
            "vfs.read_file" => Self::vfs(
                BuiltinToolOperation::ReadFile,
                BuiltinToolSurface::Canonical,
            ),
            "vfs.write_file" => Self::vfs(
                BuiltinToolOperation::WriteFile,
                BuiltinToolSurface::Canonical,
            ),
            "vfs.edit_file" => Self::vfs(
                BuiltinToolOperation::EditFile,
                BuiltinToolSurface::Canonical,
            ),
            "vfs.apply_patch" => Self::vfs(
                BuiltinToolOperation::ApplyPatch,
                BuiltinToolSurface::Canonical,
            ),
            "vfs.grep" => Self::vfs(BuiltinToolOperation::Grep, BuiltinToolSurface::Canonical),
            "vfs.glob" => Self::vfs(BuiltinToolOperation::Glob, BuiltinToolSurface::Canonical),
            "vfs.list_dir" => {
                Self::vfs(BuiltinToolOperation::ListDir, BuiltinToolSurface::Canonical)
            }
            "env.read_file" => Self::environment(
                BuiltinToolOperation::ReadFile,
                BuiltinToolSurface::Canonical,
            ),
            "env.write_file" => Self::environment(
                BuiltinToolOperation::WriteFile,
                BuiltinToolSurface::Canonical,
            ),
            "env.edit_file" => Self::environment(
                BuiltinToolOperation::EditFile,
                BuiltinToolSurface::Canonical,
            ),
            "env.apply_patch" => Self::environment(
                BuiltinToolOperation::ApplyPatch,
                BuiltinToolSurface::Canonical,
            ),
            "env.grep" => {
                Self::environment(BuiltinToolOperation::Grep, BuiltinToolSurface::Canonical)
            }
            "env.glob" => {
                Self::environment(BuiltinToolOperation::Glob, BuiltinToolSurface::Canonical)
            }
            "env.list_dir" => {
                Self::environment(BuiltinToolOperation::ListDir, BuiltinToolSurface::Canonical)
            }
            "env.run_process" => Self::environment(
                BuiltinToolOperation::RunProcess,
                BuiltinToolSurface::Canonical,
            ),
            "env.write_process_stdin" => Self::environment(
                BuiltinToolOperation::WriteProcessStdin,
                BuiltinToolSurface::Canonical,
            ),
            "env.job_submit" => Self::environment(
                BuiltinToolOperation::JobSubmit,
                BuiltinToolSurface::Canonical,
            ),
            "env.job_run" => {
                Self::environment(BuiltinToolOperation::JobRun, BuiltinToolSurface::Canonical)
            }
            "env.job_read" => {
                Self::environment(BuiltinToolOperation::JobRead, BuiltinToolSurface::Canonical)
            }
            _ => return None,
        })
    }

    pub fn from_binding(logical_id: &str, adapter_id: Option<&str>) -> Option<Self> {
        let mut tool = Self::from_logical_id(logical_id)?;
        tool.surface = match adapter_id.unwrap_or("canonical") {
            "canonical" => BuiltinToolSurface::Canonical,
            "codex" => BuiltinToolSurface::CodexLike,
            "claude" => BuiltinToolSurface::ClaudeCodeLike,
            _ => return None,
        };
        Some(tool)
    }

    pub const fn requires_write(self) -> bool {
        matches!(
            self.operation,
            BuiltinToolOperation::WriteFile
                | BuiltinToolOperation::EditFile
                | BuiltinToolOperation::ApplyPatch
        )
    }

    pub const fn requires_process(self) -> bool {
        matches!(
            self.operation,
            BuiltinToolOperation::RunProcess | BuiltinToolOperation::WriteProcessStdin
        )
    }

    pub const fn requires_jobs(self) -> bool {
        matches!(
            self.operation,
            BuiltinToolOperation::JobSubmit
                | BuiltinToolOperation::JobRun
                | BuiltinToolOperation::JobRead
        )
    }

    pub const fn is_filesystem_operation(self) -> bool {
        !self.requires_process() && !self.requires_jobs()
    }

    pub const fn parallelism(self) -> ToolParallelism {
        match self.operation {
            BuiltinToolOperation::ReadFile
            | BuiltinToolOperation::Grep
            | BuiltinToolOperation::Glob
            | BuiltinToolOperation::ListDir => ToolParallelism::ParallelSafe,
            BuiltinToolOperation::WriteFile
            | BuiltinToolOperation::EditFile
            | BuiltinToolOperation::ApplyPatch
            | BuiltinToolOperation::RunProcess
            | BuiltinToolOperation::WriteProcessStdin
            | BuiltinToolOperation::JobSubmit
            | BuiltinToolOperation::JobRun => ToolParallelism::Exclusive,
            BuiltinToolOperation::JobRead => ToolParallelism::ParallelSafe,
        }
    }

    pub const fn execution_spec(self) -> ToolExecutionSpec {
        match self.operation {
            BuiltinToolOperation::ReadFile
            | BuiltinToolOperation::Grep
            | BuiltinToolOperation::Glob
            | BuiltinToolOperation::ListDir => ToolExecutionSpec {
                class: ToolExecutionClass::Interactive,
                retry_safe: true,
            },
            BuiltinToolOperation::WriteFile
            | BuiltinToolOperation::EditFile
            | BuiltinToolOperation::ApplyPatch => ToolExecutionSpec {
                class: ToolExecutionClass::Interactive,
                retry_safe: false,
            },
            BuiltinToolOperation::RunProcess | BuiltinToolOperation::WriteProcessStdin => {
                ToolExecutionSpec {
                    class: ToolExecutionClass::Process,
                    retry_safe: false,
                }
            }
            BuiltinToolOperation::JobSubmit | BuiltinToolOperation::JobRun => ToolExecutionSpec {
                class: ToolExecutionClass::RemoteInteractive,
                retry_safe: false,
            },
            BuiltinToolOperation::JobRead => ToolExecutionSpec {
                class: ToolExecutionClass::RemoteInteractive,
                retry_safe: true,
            },
        }
    }

    pub fn binding(self, target: &ToolTarget, dispatch: ToolDispatchMode) -> ToolBinding {
        ToolBinding::new(
            self.name(target),
            self.logical_id(),
            dispatch,
            self.parallelism(),
        )
        .with_adapter_id(match self.surface {
            BuiltinToolSurface::Canonical => "canonical",
            BuiltinToolSurface::CodexLike => "codex",
            BuiltinToolSurface::ClaudeCodeLike => "claude",
        })
    }

    pub fn spec_bundle(
        self,
        target: &ToolTarget,
        scoped_paths: bool,
    ) -> ToolResult<ToolSpecBundle> {
        let description =
            ToolDocument::text("text/plain; charset=utf-8", self.description(scoped_paths)?);
        let input_schema = ToolDocument::text(
            "application/schema+json",
            serde_json::to_string(&self.input_schema(target)?).map_err(|error| {
                ToolError::InvalidRequest {
                    message: format!("failed to encode tool schema: {error}"),
                }
            })?,
        );
        Ok(ToolSpecBundle {
            spec: ToolSpec {
                name: self.name(target),
                kind: ToolKind::Function(FunctionToolSpec {
                    description_ref: Some(description.blob_ref.clone()),
                    input_schema_ref: input_schema.blob_ref.clone(),
                    output_schema_ref: None,
                    strict: Some(false),
                    provider_options_ref: None,
                }),
                parallelism: self.parallelism(),
                execution: self.execution_spec(),
            },
            documents: vec![description, input_schema],
        })
    }

    fn description(self, scoped_paths: bool) -> ToolResult<String> {
        let description = match self.surface {
            BuiltinToolSurface::Canonical => {
                Ok(canonical::description(self.operation, scoped_paths))
            }
            BuiltinToolSurface::CodexLike => Ok(codex::description(self.operation, scoped_paths)),
            BuiltinToolSurface::ClaudeCodeLike => claude::description(self.operation, scoped_paths),
        }?;
        let boundary = match self.domain {
            BuiltinToolDomain::Vfs => {
                " Accesses only session-linked VFS workspaces and snapshots; these files are not visible to environment commands."
            }
            BuiltinToolDomain::Environment if self.is_filesystem_operation() => {
                " Accesses only the active environment filesystem; it does not read or modify linked VFS files."
            }
            BuiltinToolDomain::Environment => {
                " Operates only in the active environment; linked VFS files are not implicitly available."
            }
        };
        Ok(format!("{description}{boundary}"))
    }

    fn input_schema(self, _target: &ToolTarget) -> ToolResult<Value> {
        match self.surface {
            BuiltinToolSurface::Canonical => Ok(canonical::input_schema(self.operation)),
            BuiltinToolSurface::CodexLike => Ok(codex::input_schema(self.operation)),
            BuiltinToolSurface::ClaudeCodeLike => claude::input_schema(self.operation),
        }
    }

    pub async fn invoke_json(
        self,
        ctx: BuiltinToolContext<'_>,
        arguments: Value,
    ) -> ToolResult<ToolInvocationOutput> {
        match self.surface {
            BuiltinToolSurface::Canonical => {
                canonical::invoke_json(self.operation, ctx, arguments).await
            }
            BuiltinToolSurface::CodexLike => {
                codex::invoke_json(self.operation, ctx, arguments).await
            }
            BuiltinToolSurface::ClaudeCodeLike => {
                claude::invoke_json(self.operation, ctx, arguments).await
            }
        }
    }
}

impl BuiltinToolOperation {
    pub(super) fn name_for_error(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::EditFile => "edit_file",
            Self::ApplyPatch => "apply_patch",
            Self::Grep => "grep",
            Self::Glob => "glob",
            Self::ListDir => "list_dir",
            Self::RunProcess => "run_process",
            Self::WriteProcessStdin => "write_process_stdin",
            Self::JobSubmit => "job_submit",
            Self::JobRun => "job_run",
            Self::JobRead => "job_read",
        }
    }
}

#[cfg(test)]
mod tests {
    use engine::{ProviderApiKind, ToolKind};
    use serde_json::json;

    use super::*;
    use crate::{fs::FsPath, runtime::decode_args};

    fn target() -> ToolTarget {
        ToolTarget::api_kind(ProviderApiKind::OpenAiResponses)
    }

    #[test]
    fn built_in_tool_names_are_valid_tool_names() {
        for tool in [
            BuiltinTool::environment_canonical(BuiltinToolOperation::ReadFile),
            BuiltinTool::environment_canonical(BuiltinToolOperation::WriteFile),
            BuiltinTool::environment_canonical(BuiltinToolOperation::EditFile),
            BuiltinTool::environment_canonical(BuiltinToolOperation::ApplyPatch),
            BuiltinTool::environment_canonical(BuiltinToolOperation::Grep),
            BuiltinTool::environment_canonical(BuiltinToolOperation::Glob),
            BuiltinTool::environment_canonical(BuiltinToolOperation::ListDir),
            BuiltinTool::environment_canonical(BuiltinToolOperation::RunProcess),
            BuiltinTool::environment_canonical(BuiltinToolOperation::WriteProcessStdin),
            BuiltinTool::environment_canonical(BuiltinToolOperation::JobSubmit),
            BuiltinTool::environment_canonical(BuiltinToolOperation::JobRun),
        ] {
            assert_eq!(tool.name(&target()).as_str(), tool.name_str());
        }
    }

    #[test]
    fn job_submit_replaces_job_start_without_an_alias() {
        let tool = BuiltinTool::from_logical_id("env.job_submit").expect("job_submit logical id");

        assert_eq!(tool.operation(), BuiltinToolOperation::JobSubmit);
        assert_eq!(tool.name_str(), "job_submit");
        assert!(BuiltinTool::from_logical_id("env.job_start").is_none());
        assert!(BuiltinTool::from_logical_id("host.job_start").is_none());
    }

    #[test]
    fn job_run_schema_is_flat_single_job_work() {
        let schema = BuiltinTool::environment_canonical(BuiltinToolOperation::JobRun)
            .input_schema(&target())
            .expect("job_run schema");

        assert_eq!(schema["required"], json!(["argv"]));
        assert!(schema["properties"].get("jobs").is_none());
        assert!(schema["properties"].get("job_id").is_none());
        assert!(schema["properties"].get("depends_on").is_none());
        assert_eq!(
            schema["properties"]["timeout_ms"]["maximum"],
            crate::environment::jobs::JOB_RUN_MAX_TIMEOUT_MS
        );
    }

    #[test]
    fn spec_bundle_uses_content_addressed_documents() {
        let bundle = BuiltinTool::environment_canonical(BuiltinToolOperation::ReadFile)
            .spec_bundle(&target(), true)
            .expect("spec bundle");

        let ToolKind::Function(function) = bundle.spec.kind else {
            panic!("expected function tool");
        };
        assert_eq!(bundle.documents.len(), 2);
        assert_eq!(
            function.description_ref,
            Some(bundle.documents[0].blob_ref.clone())
        );
        assert_eq!(function.input_schema_ref, bundle.documents[1].blob_ref);
        assert!(
            bundle.documents[0]
                .text_lossy()
                .contains("configured filesystem scope")
        );
        assert!(bundle.documents[1].text_lossy().contains("\"path\""));
    }

    #[test]
    fn process_tools_use_stable_environment_logical_ids() {
        let tool = BuiltinTool::environment_canonical(BuiltinToolOperation::RunProcess);
        assert_eq!(tool.domain(), BuiltinToolDomain::Environment);
        assert_eq!(tool.logical_id(), "env.run_process");
    }

    #[test]
    fn claude_code_like_surface_generates_claude_style_schema() {
        let tool = BuiltinTool::environment(
            BuiltinToolOperation::ReadFile,
            BuiltinToolSurface::ClaudeCodeLike,
        );

        assert_eq!(tool.name_str(), "Read");
        let bundle = tool.spec_bundle(&target(), false).expect("spec bundle");
        assert!(bundle.documents[1].text_lossy().contains("\"file_path\""));
    }

    #[test]
    fn claude_code_like_surface_supports_list_dir_in_both_domains() {
        let vfs_tool = BuiltinTool::vfs(
            BuiltinToolOperation::ListDir,
            BuiltinToolSurface::ClaudeCodeLike,
        );
        let environment_tool = BuiltinTool::environment(
            BuiltinToolOperation::ListDir,
            BuiltinToolSurface::ClaudeCodeLike,
        );

        assert_eq!(vfs_tool.name_str(), "VfsListDir");
        assert_eq!(environment_tool.name_str(), "ListDir");
        for tool in [vfs_tool, environment_tool] {
            let bundle = tool.spec_bundle(&target(), false).expect("spec bundle");
            assert!(bundle.documents[1].text_lossy().contains("\"path\""));
        }
    }

    #[test]
    fn claude_code_like_surface_rejects_unmapped_operations() {
        let tool = BuiltinTool::environment(
            BuiltinToolOperation::ApplyPatch,
            BuiltinToolSurface::ClaudeCodeLike,
        );

        assert!(matches!(
            tool.spec_bundle(&target(), false),
            Err(ToolError::UnsupportedCapability { .. })
        ));
    }

    #[test]
    fn canonical_args_default_model_omitted_convenience_fields() {
        let list: ListDirArgs = decode_args(json!({})).expect("list args");
        assert_eq!(list.path, FsPath::root());

        let grep: GrepArgs = decode_args(json!({ "pattern": "struct Foo" })).expect("grep args");
        assert!(!grep.case_sensitive);

        let edit: EditFileArgs = decode_args(json!({
            "path": "src/lib.rs",
            "old_string": "before",
            "new_string": "after"
        }))
        .expect("edit args");
        assert!(!edit.replace_all);

        let run: RunProcessArgs =
            decode_args(json!({ "argv": ["cargo", "test"] })).expect("run args");
        assert!(run.env.is_empty());

        let stdin: WriteProcessStdinArgs =
            decode_args(json!({ "handle": "proc-1", "input": "q" })).expect("stdin args");
        assert_eq!(stdin.handle.as_str(), "proc-1");
        assert!(!stdin.close_stdin);
    }
}
