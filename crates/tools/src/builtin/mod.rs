//! Built-in filesystem and environment action tool definitions.
//!
//! A built-in tool is one substrate operation presented on one surface. The
//! substrate is the same for every provider; a surface is a mapping table
//! plus text, and never changes what the substrate does. A surface may
//! present one operation as more than one tool (Claude Code's `BashOutput`
//! and `KillShell` are both `continue_process`), and a toolset may restrict
//! the process tools to a one-shot shape that omits the handle path.

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
    ContinueProcessArgs, RunProcessArgs, invoke_continue_process, invoke_job_read,
    invoke_job_submit, invoke_run_process,
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
    ContinueProcess,
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

/// Which of a surface's presentations of an operation this tool is. Only
/// the Claude-Code-like `continue_process` has more than one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BuiltinToolVariant {
    Primary,
    /// `KillShell`: `continue_process` with `signal: kill`.
    Kill,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BuiltinTool {
    domain: BuiltinToolDomain,
    operation: BuiltinToolOperation,
    surface: BuiltinToolSurface,
    variant: BuiltinToolVariant,
    /// The toolset omits the handle path: the run tool is rendered in its
    /// restricted shape and the continue tool is absent.
    one_shot: bool,
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

const ADAPTER_CANONICAL: &str = "canonical";
const ADAPTER_CODEX: &str = "codex";
const ADAPTER_CLAUDE: &str = "claude";
const ADAPTER_ONE_SHOT_SUFFIX: &str = "-oneshot";

impl BuiltinTool {
    pub const fn environment(operation: BuiltinToolOperation, surface: BuiltinToolSurface) -> Self {
        Self {
            domain: BuiltinToolDomain::Environment,
            operation,
            surface,
            variant: BuiltinToolVariant::Primary,
            one_shot: false,
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
            variant: BuiltinToolVariant::Primary,
            one_shot: false,
        }
    }

    pub const fn with_one_shot(mut self, one_shot: bool) -> Self {
        self.one_shot = one_shot;
        self
    }

    const fn kill_variant(mut self) -> Self {
        self.variant = BuiltinToolVariant::Kill;
        self
    }

    /// Every tool this surface renders for the operation: the primary one,
    /// plus `KillShell` for the Claude-Code-like continue operation.
    pub fn variants(self) -> Vec<Self> {
        if self.has_kill_variant() {
            vec![self, self.kill_variant()]
        } else {
            vec![self]
        }
    }

    const fn has_kill_variant(self) -> bool {
        matches!(
            (self.domain, self.surface, self.operation),
            (
                BuiltinToolDomain::Environment,
                BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::ContinueProcess
            )
        )
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

    pub const fn variant(self) -> BuiltinToolVariant {
        self.variant
    }

    pub const fn one_shot(self) -> bool {
        self.one_shot
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
            (BuiltinToolDomain::Environment, BuiltinToolOperation::ContinueProcess) => {
                "env.continue_process"
            }
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobSubmit) => "env.job_submit",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobRun) => "env.job_run",
            (BuiltinToolDomain::Environment, BuiltinToolOperation::JobRead) => "env.job_read",
            (
                BuiltinToolDomain::Vfs,
                BuiltinToolOperation::RunProcess
                | BuiltinToolOperation::ContinueProcess
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
                    | BuiltinToolOperation::ContinueProcess
                    | BuiltinToolOperation::JobSubmit
                    | BuiltinToolOperation::JobRun
                    | BuiltinToolOperation::JobRead,
                ) => unreachable!(),
            };
        }
        match (self.surface, self.operation, self.variant) {
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ReadFile,
                _,
            ) => "read_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::WriteFile,
                _,
            ) => "write_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::EditFile,
                _,
            ) => "edit_file",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ApplyPatch,
                _,
            ) => "apply_patch",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::Grep,
                _,
            ) => "grep",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::Glob,
                _,
            ) => "glob",
            (
                BuiltinToolSurface::Canonical | BuiltinToolSurface::CodexLike,
                BuiltinToolOperation::ListDir,
                _,
            ) => "list_dir",
            (BuiltinToolSurface::Canonical, BuiltinToolOperation::RunProcess, _) => "run_process",
            (BuiltinToolSurface::Canonical, BuiltinToolOperation::ContinueProcess, _) => {
                "continue_process"
            }
            (BuiltinToolSurface::CodexLike, BuiltinToolOperation::RunProcess, _) => "exec_command",
            (BuiltinToolSurface::CodexLike, BuiltinToolOperation::ContinueProcess, _) => {
                "write_stdin"
            }
            (_, BuiltinToolOperation::JobSubmit, _) => {
                crate::environment::jobs::JOB_SUBMIT_TOOL_NAME
            }
            (_, BuiltinToolOperation::JobRun, _) => crate::environment::jobs::JOB_RUN_TOOL_NAME,
            (_, BuiltinToolOperation::JobRead, _) => crate::environment::jobs::JOB_READ_TOOL_NAME,
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ReadFile, _) => "Read",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::WriteFile, _) => "Write",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::EditFile, _) => "Edit",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Grep, _) => "Grep",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::Glob, _) => "Glob",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::RunProcess, _) => "Bash",
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ApplyPatch, _) => {
                "apply_patch"
            }
            (BuiltinToolSurface::ClaudeCodeLike, BuiltinToolOperation::ListDir, _) => "ListDir",
            (
                BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::ContinueProcess,
                BuiltinToolVariant::Primary,
            ) => "BashOutput",
            (
                BuiltinToolSurface::ClaudeCodeLike,
                BuiltinToolOperation::ContinueProcess,
                BuiltinToolVariant::Kill,
            ) => "KillShell",
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
            "env.continue_process" => Self::environment(
                BuiltinToolOperation::ContinueProcess,
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

    /// Resolves a catalog binding back to the tool that rendered it. The
    /// adapter id carries the surface and the one-shot policy; the tool name
    /// picks the variant when a surface renders the operation twice.
    pub fn from_binding(
        logical_id: &str,
        adapter_id: Option<&str>,
        tool_name: &str,
    ) -> Option<Self> {
        let mut tool = Self::from_logical_id(logical_id)?;
        let adapter_id = adapter_id.unwrap_or(ADAPTER_CANONICAL);
        let (surface_id, one_shot) = match adapter_id.strip_suffix(ADAPTER_ONE_SHOT_SUFFIX) {
            Some(surface_id) => (surface_id, true),
            None => (adapter_id, false),
        };
        tool.surface = match surface_id {
            ADAPTER_CANONICAL => BuiltinToolSurface::Canonical,
            ADAPTER_CODEX => BuiltinToolSurface::CodexLike,
            ADAPTER_CLAUDE => BuiltinToolSurface::ClaudeCodeLike,
            _ => return None,
        };
        tool.one_shot = one_shot;
        if tool.has_kill_variant() && tool.kill_variant().name_str() == tool_name {
            tool = tool.kill_variant();
        }
        Some(tool)
    }

    fn adapter_id(self) -> String {
        let surface = match self.surface {
            BuiltinToolSurface::Canonical => ADAPTER_CANONICAL,
            BuiltinToolSurface::CodexLike => ADAPTER_CODEX,
            BuiltinToolSurface::ClaudeCodeLike => ADAPTER_CLAUDE,
        };
        if self.one_shot {
            format!("{surface}{ADAPTER_ONE_SHOT_SUFFIX}")
        } else {
            surface.to_owned()
        }
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
            BuiltinToolOperation::RunProcess | BuiltinToolOperation::ContinueProcess
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
            | BuiltinToolOperation::ContinueProcess
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
            BuiltinToolOperation::RunProcess | BuiltinToolOperation::ContinueProcess => {
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
        .with_adapter_id(self.adapter_id())
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
            BuiltinToolSurface::Canonical => Ok(canonical::description(self, scoped_paths)),
            BuiltinToolSurface::CodexLike => Ok(codex::description(self, scoped_paths)),
            BuiltinToolSurface::ClaudeCodeLike => claude::description(self, scoped_paths),
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
            BuiltinToolSurface::Canonical => Ok(canonical::input_schema(self)),
            BuiltinToolSurface::CodexLike => Ok(codex::input_schema(self)),
            BuiltinToolSurface::ClaudeCodeLike => claude::input_schema(self),
        }
    }

    pub async fn invoke_json(
        self,
        ctx: BuiltinToolContext<'_>,
        arguments: Value,
    ) -> ToolResult<ToolInvocationOutput> {
        match self.surface {
            BuiltinToolSurface::Canonical => canonical::invoke_json(self, ctx, arguments).await,
            BuiltinToolSurface::CodexLike => codex::invoke_json(self, ctx, arguments).await,
            BuiltinToolSurface::ClaudeCodeLike => claude::invoke_json(self, ctx, arguments).await,
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
            Self::ContinueProcess => "continue_process",
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
            BuiltinTool::environment_canonical(BuiltinToolOperation::ContinueProcess),
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
    fn process_tools_use_stable_environment_logical_ids_on_every_surface() {
        for surface in [
            BuiltinToolSurface::Canonical,
            BuiltinToolSurface::CodexLike,
            BuiltinToolSurface::ClaudeCodeLike,
        ] {
            let run = BuiltinTool::environment(BuiltinToolOperation::RunProcess, surface);
            assert_eq!(run.domain(), BuiltinToolDomain::Environment);
            assert_eq!(run.logical_id(), "env.run_process");
            let cont = BuiltinTool::environment(BuiltinToolOperation::ContinueProcess, surface);
            assert_eq!(cont.logical_id(), "env.continue_process");
        }
        assert!(BuiltinTool::from_logical_id("env.write_process_stdin").is_none());
    }

    #[test]
    fn surface_names_for_process_tools() {
        let names = |surface| {
            BuiltinTool::environment(BuiltinToolOperation::ContinueProcess, surface)
                .variants()
                .into_iter()
                .map(|tool| tool.name_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            BuiltinTool::environment(
                BuiltinToolOperation::RunProcess,
                BuiltinToolSurface::Canonical
            )
            .name_str(),
            "run_process"
        );
        assert_eq!(
            BuiltinTool::environment(
                BuiltinToolOperation::RunProcess,
                BuiltinToolSurface::CodexLike
            )
            .name_str(),
            "exec_command"
        );
        assert_eq!(
            BuiltinTool::environment(
                BuiltinToolOperation::RunProcess,
                BuiltinToolSurface::ClaudeCodeLike
            )
            .name_str(),
            "Bash"
        );
        assert_eq!(names(BuiltinToolSurface::Canonical), ["continue_process"]);
        assert_eq!(names(BuiltinToolSurface::CodexLike), ["write_stdin"]);
        assert_eq!(
            names(BuiltinToolSurface::ClaudeCodeLike),
            ["BashOutput", "KillShell"]
        );
    }

    #[test]
    fn bindings_round_trip_surface_variant_and_one_shot_policy() {
        let target = target();
        let kill = BuiltinTool::environment(
            BuiltinToolOperation::ContinueProcess,
            BuiltinToolSurface::ClaudeCodeLike,
        )
        .kill_variant();
        let binding = kill.binding(&target, ToolDispatchMode::Local);
        assert_eq!(binding.tool_name.as_str(), "KillShell");
        assert_eq!(binding.adapter_id.as_deref(), Some("claude"));
        let resolved = BuiltinTool::from_binding(
            &binding.logical_id,
            binding.adapter_id.as_deref(),
            binding.tool_name.as_str(),
        )
        .expect("binding resolves");
        assert_eq!(resolved, kill);
        assert_eq!(resolved.variant(), BuiltinToolVariant::Kill);

        let one_shot = BuiltinTool::environment(
            BuiltinToolOperation::RunProcess,
            BuiltinToolSurface::CodexLike,
        )
        .with_one_shot(true);
        let binding = one_shot.binding(&target, ToolDispatchMode::Local);
        assert_eq!(binding.adapter_id.as_deref(), Some("codex-oneshot"));
        let resolved = BuiltinTool::from_binding(
            &binding.logical_id,
            binding.adapter_id.as_deref(),
            binding.tool_name.as_str(),
        )
        .expect("binding resolves");
        assert!(resolved.one_shot());
        assert_eq!(resolved.surface(), BuiltinToolSurface::CodexLike);
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
        assert!(!run.tty);
        assert_eq!(run.timeout_ms, None);

        let cont: ContinueProcessArgs =
            decode_args(json!({ "handle": "proc-1" })).expect("continue args");
        assert_eq!(cont.handle.as_str(), "proc-1");
        assert!(!cont.close_stdin);
        assert_eq!(cont.input, None);
        assert_eq!(cont.signal, None);
    }
}
