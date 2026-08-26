//! Canonical Lightspeed built-in tool surface.

use serde_json::{Value, json};

use crate::{
    environment::{
        jobs::{
            JOB_READ_TOOL_NAME, JOB_RUN_MAX_TIMEOUT_MS, JOB_RUN_TOOL_NAME, JOB_SUBMIT_TOOL_NAME,
            visible_job_read_output,
        },
        tools::{
            invoke_job_read, invoke_job_submit, invoke_run_process, invoke_write_process_stdin,
        },
    },
    error::ToolResult,
    fs::tools::{
        invoke_apply_patch, invoke_edit_file, invoke_glob, invoke_grep, invoke_list_dir,
        invoke_read_file, invoke_write_file,
    },
    runtime::{ToolInvocationOutput, decode_args, encode_output},
};

use super::{
    BuiltinToolContext, BuiltinToolOperation,
    shared::{
        array_of_strings, boolean, nullable_integer, nullable_string, object,
        process_visible_output, string, string_map, visible_with_search_stop,
    },
};

pub(super) fn description(operation: BuiltinToolOperation, scoped_paths: bool) -> String {
    let path_guidance = if scoped_paths {
        " Paths are resolved within the configured filesystem scope."
    } else {
        ""
    };
    let text = match operation {
        BuiltinToolOperation::ReadFile => {
            "Read a UTF-8 file with optional 1-based line offset and line limit."
        }
        BuiltinToolOperation::WriteFile => {
            "Write full UTF-8 file content, creating parent directories when needed."
        }
        BuiltinToolOperation::EditFile => {
            "Replace exact text in a UTF-8 file. Multiple matches require replace_all=true."
        }
        BuiltinToolOperation::ApplyPatch => {
            "Apply a Codex-style apply_patch patch to the filesystem."
        }
        BuiltinToolOperation::Grep => "Search UTF-8 files recursively with a regular expression.",
        BuiltinToolOperation::Glob => "Find files recursively with a glob pattern.",
        BuiltinToolOperation::ListDir => "List one directory.",
        BuiltinToolOperation::RunProcess => {
            "Run a process through the configured process executor."
        }
        BuiltinToolOperation::WriteProcessStdin => "Write input to an existing process handle.",
        BuiltinToolOperation::JobSubmit => {
            "Start one or more durable environment jobs asynchronously. Returns one Promise per job; use await, cancel, or detach when appropriate."
        }
        BuiltinToolOperation::JobRun => {
            "Run one durable environment job and wait for its terminal readable result. Use job_submit for dependency groups, longer work, or explicit Promise control."
        }
        BuiltinToolOperation::JobRead => {
            "Read durable environment job status and bounded output tails."
        }
    };
    format!("{text}{path_guidance}")
}

pub(super) fn input_schema(operation: BuiltinToolOperation) -> Value {
    match operation {
        BuiltinToolOperation::ReadFile => object(
            [
                ("path", string("File path to read.")),
                (
                    "offset",
                    nullable_integer("1-based line number to start at."),
                ),
                (
                    "limit",
                    nullable_integer("Maximum number of lines to return."),
                ),
            ],
            ["path"],
        ),
        BuiltinToolOperation::WriteFile => object(
            [
                ("path", string("File path to write.")),
                ("content", string("Full file content.")),
            ],
            ["path", "content"],
        ),
        BuiltinToolOperation::EditFile => object(
            [
                ("path", string("File path to edit.")),
                ("old_string", string("Exact text to replace.")),
                ("new_string", string("Replacement text.")),
                (
                    "replace_all",
                    boolean(
                        "Replace all matches instead of requiring one match. Defaults to false.",
                    ),
                ),
            ],
            ["path", "old_string", "new_string"],
        ),
        BuiltinToolOperation::ApplyPatch => object(
            [(
                "patch",
                string("Full apply_patch text, including begin and end markers."),
            )],
            ["patch"],
        ),
        BuiltinToolOperation::Grep => object(
            [
                ("pattern", string("Regular expression to search for.")),
                ("path", nullable_string("Directory path to search from.")),
                (
                    "include",
                    nullable_string("Optional glob for files to include."),
                ),
                (
                    "case_sensitive",
                    boolean("Whether the regex is case-sensitive. Defaults to false."),
                ),
                (
                    "max_depth",
                    nullable_integer("Optional maximum directory depth."),
                ),
                (
                    "limit",
                    nullable_integer("Maximum number of matching lines."),
                ),
            ],
            ["pattern"],
        ),
        BuiltinToolOperation::Glob => object(
            [
                ("pattern", string("Glob pattern to match files.")),
                ("path", nullable_string("Directory path to search from.")),
                (
                    "max_depth",
                    nullable_integer("Optional maximum directory depth."),
                ),
                (
                    "limit",
                    nullable_integer("Maximum number of matching files."),
                ),
            ],
            ["pattern"],
        ),
        BuiltinToolOperation::ListDir => object(
            [(
                "path",
                string("Directory path to list. Defaults to the workspace root."),
            )],
            [],
        ),
        BuiltinToolOperation::RunProcess => object(
            [
                ("argv", array_of_strings("Process argv.")),
                (
                    "cwd",
                    nullable_string("Optional process working directory."),
                ),
                (
                    "env",
                    string_map("Environment variables. Defaults to empty."),
                ),
                ("stdin", nullable_string("Optional standard input.")),
                (
                    "timeout_ms",
                    nullable_integer(
                        "Optional timeout in milliseconds. Defaults to 60 seconds; requests above the deployment ceiling (default 30 minutes) are clamped.",
                    ),
                ),
                (
                    "yield_time_ms",
                    nullable_integer("Optional yield interval in milliseconds."),
                ),
                (
                    "max_output_bytes",
                    nullable_integer("Optional output byte limit."),
                ),
            ],
            ["argv"],
        ),
        BuiltinToolOperation::WriteProcessStdin => object(
            [
                ("handle", string("Process handle.")),
                ("input", string("Input to write.")),
                (
                    "close_stdin",
                    boolean("Whether to close stdin after writing. Defaults to false."),
                ),
                (
                    "yield_time_ms",
                    nullable_integer("Optional yield interval in milliseconds."),
                ),
                (
                    "max_output_bytes",
                    nullable_integer("Optional output byte limit."),
                ),
            ],
            ["handle", "input"],
        ),
        BuiltinToolOperation::JobSubmit => job_submit_schema(),
        BuiltinToolOperation::JobRun => job_run_schema(),
        BuiltinToolOperation::JobRead => job_read_schema(),
    }
}

pub(super) async fn invoke_json(
    operation: BuiltinToolOperation,
    ctx: BuiltinToolContext<'_>,
    arguments: Value,
) -> ToolResult<ToolInvocationOutput> {
    match operation {
        BuiltinToolOperation::ReadFile => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_read_file(fs_ctx, decode_args(arguments)?).await?;
            encode_output(&result, result.line_numbered_text.clone())
        }
        BuiltinToolOperation::WriteFile => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_write_file(fs_ctx, decode_args(arguments)?).await?;
            let visible = format!(
                "Wrote {} bytes to {}",
                result.bytes_written, result.resolved_path
            );
            encode_output(&result, visible)
        }
        BuiltinToolOperation::EditFile => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_edit_file(fs_ctx, decode_args(arguments)?).await?;
            let visible = format!(
                "Replaced {} match(es) in {}",
                result.replacements, result.resolved_path
            );
            encode_output(&result, visible)
        }
        BuiltinToolOperation::ApplyPatch => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_apply_patch(fs_ctx, decode_args(arguments)?).await?;
            encode_output(&result, result.output.clone())
        }
        BuiltinToolOperation::Grep => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_grep(fs_ctx, decode_args(arguments)?).await?;
            let visible = result
                .matches
                .iter()
                .map(|m| format!("{}:{}:{}", m.path, m.line_number, m.line))
                .collect::<Vec<_>>()
                .join("\n");
            encode_output(&result, visible_with_search_stop(visible, result.stopped))
        }
        BuiltinToolOperation::Glob => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_glob(fs_ctx, decode_args(arguments)?).await?;
            let visible = result
                .matches
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            encode_output(&result, visible_with_search_stop(visible, result.stopped))
        }
        BuiltinToolOperation::ListDir => {
            let fs_ctx = ctx.filesystem()?;
            let result = invoke_list_dir(fs_ctx, decode_args(arguments)?).await?;
            let visible = result
                .entries
                .iter()
                .map(|entry| {
                    let suffix = if entry.is_directory { "/" } else { "" };
                    format!("{}{suffix}", entry.file_name)
                })
                .collect::<Vec<_>>()
                .join("\n");
            encode_output(&result, visible)
        }
        BuiltinToolOperation::RunProcess => {
            let env_ctx = ctx.environment()?;
            let result = invoke_run_process(env_ctx, decode_args(arguments)?).await?;
            let visible = process_visible_output(&result);
            encode_output(&result, visible)
        }
        BuiltinToolOperation::WriteProcessStdin => {
            let env_ctx = ctx.environment()?;
            let result = invoke_write_process_stdin(env_ctx, decode_args(arguments)?).await?;
            let visible = process_visible_output(&result);
            encode_output(&result, visible)
        }
        BuiltinToolOperation::JobSubmit => {
            let env_ctx = ctx.environment()?;
            let result = invoke_job_submit(env_ctx, decode_args(arguments)?).await?;
            let visible = result
                .jobs
                .iter()
                .map(|job| format!("{}: {:?}", job.job_id.as_str(), job.status))
                .collect::<Vec<_>>()
                .join("\n");
            encode_output(&result, visible)
        }
        BuiltinToolOperation::JobRun => Err(crate::error::ToolError::UnsupportedCapability {
            message: "job_run requires its joined workflow-tool binding".to_owned(),
        }),
        BuiltinToolOperation::JobRead => {
            let env_ctx = ctx.environment()?;
            let result = invoke_job_read(env_ctx, decode_args(arguments)?).await?;
            let visible = visible_job_read_output(&result.jobs);
            encode_output(&result, visible)
        }
    }
}

fn job_submit_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "jobs": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": ["string", "null"] },
                        "job_id": {
                            "type": "string",
                            "pattern": "^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$",
                            "description": "Your name for the job (for example \"build\"). It keys the job's promise in the result and identifies the job to job_read."
                        },
                        "argv": { "type": "array", "items": { "type": "string" } },
                        "cwd": { "type": ["string", "null"] },
                        "env": { "type": "object", "additionalProperties": { "type": "string" } },
                        "stdin": { "type": ["string", "null"] },
                        "timeout_ms": { "type": ["integer", "null"], "minimum": 0 },
                        "depends_on": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "job_id": { "type": ["string", "null"] },
                                    "name": { "type": ["string", "null"] }
                                },
                                "additionalProperties": false
                            }
                        },
                        "dependency_policy": { "type": "string", "enum": ["allSucceeded", "allTerminal"] },
                        "queue_key": { "type": ["string", "null"] }
                    },
                    "required": ["job_id", "argv"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["jobs"],
        "additionalProperties": false,
        "description": JOB_SUBMIT_TOOL_NAME
    })
}

fn job_run_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": ["string", "null"] },
            "argv": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "description": "Process argv for the single durable job."
            },
            "cwd": { "type": ["string", "null"] },
            "env": { "type": "object", "additionalProperties": { "type": "string" } },
            "stdin": { "type": ["string", "null"] },
            "timeout_ms": {
                "type": ["integer", "null"],
                "minimum": 0,
                "maximum": JOB_RUN_MAX_TIMEOUT_MS,
                "description": "Provider execution timeout. Defaults to 30 minutes; maximum 60 minutes."
            },
            "queue_key": { "type": ["string", "null"] }
        },
        "required": ["argv"],
        "additionalProperties": false,
        "description": JOB_RUN_TOOL_NAME
    })
}

fn job_handle_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "environment_id": {
                "type": ["string", "null"],
                "description": "Omit for this session's active environment (where job_submit and job_run start jobs); set it only to read a job in another environment."
            },
            "job_id": { "type": "string", "description": "The job id you gave job_submit, or the one job_run returned." }
        },
        "required": ["job_id"],
        "additionalProperties": false
    })
}

fn job_read_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "jobs": { "type": "array", "items": job_handle_schema() },
            "output_bytes": { "type": ["integer", "null"], "minimum": 0 },
            "after_seq": { "type": ["integer", "null"], "minimum": 0 },
            "include_artifacts": { "type": "boolean" }
        },
        "required": ["jobs"],
        "additionalProperties": false,
        "description": JOB_READ_TOOL_NAME
    })
}
