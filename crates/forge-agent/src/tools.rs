use crate::{
    AgentError, EventEmitter, ExecutionEnvironment, GrepOptions, SessionConfig, SessionEvent,
    ToolError, truncate_tool_output,
};
use forge_llm::{ToolCall, ToolDefinition, ToolResult};
use futures::future::join_all;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, AgentError>> + Send>>;
pub type ToolExecutor =
    Arc<dyn Fn(Value, Arc<dyn ExecutionEnvironment>) -> ToolFuture + Send + Sync>;

#[derive(Clone)]
pub struct ToolDispatchOptions {
    pub session_id: String,
    pub supports_parallel_tool_calls: bool,
}

#[derive(Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    pub executor: ToolExecutor,
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: RegisteredTool) {
        self.tools.insert(tool.definition.name.clone(), tool);
    }

    pub fn unregister(&mut self, name: &str) -> Option<RegisteredTool> {
        self.tools.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions: Vec<ToolDefinition> = self
            .tools
            .values()
            .map(|tool| tool.definition.clone())
            .collect();
        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        definitions
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    pub async fn dispatch(
        &self,
        tool_calls: Vec<ToolCall>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        config: &SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        options: ToolDispatchOptions,
    ) -> Result<Vec<ToolResult>, AgentError> {
        if options.supports_parallel_tool_calls && tool_calls.len() > 1 {
            let futures = tool_calls.into_iter().map(|tool_call| {
                self.dispatch_single(
                    tool_call,
                    execution_env.clone(),
                    config,
                    event_emitter.clone(),
                    &options.session_id,
                )
            });
            return Ok(join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?);
        }

        let mut results = Vec::with_capacity(tool_calls.len());
        for tool_call in tool_calls {
            results.push(
                self.dispatch_single(
                    tool_call,
                    execution_env.clone(),
                    config,
                    event_emitter.clone(),
                    &options.session_id,
                )
                .await?,
            );
        }
        Ok(results)
    }

    async fn dispatch_single(
        &self,
        tool_call: ToolCall,
        execution_env: Arc<dyn ExecutionEnvironment>,
        config: &SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        session_id: &str,
    ) -> Result<ToolResult, AgentError> {
        event_emitter.emit(SessionEvent::tool_call_start(
            session_id.to_string(),
            tool_call.name.clone(),
            tool_call.id.clone(),
        ))?;

        let Some(registered) = self.get(&tool_call.name) else {
            let message = format!("Unknown tool: {}", tool_call.name);
            event_emitter.emit(SessionEvent::tool_call_end_error(
                session_id.to_string(),
                tool_call.id.clone(),
                message.clone(),
            ))?;
            return Ok(tool_error_result(tool_call.id, message));
        };

        let parsed_arguments = match parse_tool_arguments(&tool_call) {
            Ok(arguments) => arguments,
            Err(error) => {
                event_emitter.emit(SessionEvent::tool_call_end_error(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    error.to_string(),
                ))?;
                return Ok(tool_error_result(tool_call.id, error.to_string()));
            }
        };
        let parsed_arguments = normalize_tool_arguments_for_dispatch(
            &tool_call.name,
            parsed_arguments,
            &registered.definition.parameters,
            config,
        );

        if let Err(error) =
            validate_tool_arguments(&registered.definition.parameters, &parsed_arguments)
        {
            event_emitter.emit(SessionEvent::tool_call_end_error(
                session_id.to_string(),
                tool_call.id.clone(),
                error.to_string(),
            ))?;
            return Ok(tool_error_result(tool_call.id, error.to_string()));
        }

        let raw_output = match (registered.executor)(parsed_arguments, execution_env).await {
            Ok(output) => output,
            Err(error) => {
                let error_text = error.to_string();
                event_emitter.emit(SessionEvent::tool_call_end_error(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    error_text.clone(),
                ))?;
                return Ok(tool_error_result(tool_call.id, error_text));
            }
        };

        if !raw_output.is_empty() {
            event_emitter.emit(SessionEvent::tool_call_output_delta(
                session_id.to_string(),
                tool_call.id.clone(),
                raw_output.clone(),
            ))?;
        }
        let truncated = truncate_tool_output(&raw_output, &tool_call.name, config);
        event_emitter.emit(SessionEvent::tool_call_end_output(
            session_id.to_string(),
            tool_call.id.clone(),
            raw_output,
        ))?;

        Ok(ToolResult {
            tool_call_id: tool_call.id,
            content: Value::String(truncated),
            is_error: false,
        })
    }
}

fn normalize_tool_arguments_for_dispatch(
    tool_name: &str,
    arguments: Value,
    schema: &Value,
    config: &SessionConfig,
) -> Value {
    if tool_name != SHELL_TOOL {
        return arguments;
    }

    let has_timeout_property = schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("timeout_ms"))
        .is_some();
    if !has_timeout_property {
        return arguments;
    }

    let Some(object) = arguments.as_object() else {
        return arguments;
    };
    let mut normalized = object.clone();
    let (default_timeout_ms, max_timeout_ms) = effective_shell_timeout_policy(config);

    let timeout_ms = match normalized.get("timeout_ms") {
        Some(Value::Number(number)) => {
            if let Some(value) = number.as_u64() {
                value.min(max_timeout_ms)
            } else {
                return Value::Object(normalized);
            }
        }
        Some(_) => return Value::Object(normalized),
        None => default_timeout_ms,
    };

    normalized.insert("timeout_ms".to_string(), Value::from(timeout_ms));
    Value::Object(normalized)
}

fn effective_shell_timeout_policy(config: &SessionConfig) -> (u64, u64) {
    let default_timeout_ms = if config.default_command_timeout_ms == 0 {
        10_000
    } else {
        config.default_command_timeout_ms
    };
    let max_timeout_ms = if config.max_command_timeout_ms == 0 {
        600_000
    } else {
        config.max_command_timeout_ms
    };
    let max_timeout_ms = max_timeout_ms.max(default_timeout_ms);
    (default_timeout_ms, max_timeout_ms)
}

pub const READ_FILE_TOOL: &str = "read_file";
pub const WRITE_FILE_TOOL: &str = "write_file";
pub const EDIT_FILE_TOOL: &str = "edit_file";
pub const APPLY_PATCH_TOOL: &str = "apply_patch";
pub const SHELL_TOOL: &str = "shell";
pub const GREP_TOOL: &str = "grep";
pub const GLOB_TOOL: &str = "glob";
pub const SPAWN_AGENT_TOOL: &str = "spawn_agent";
pub const SEND_INPUT_TOOL: &str = "send_input";
pub const WAIT_TOOL: &str = "wait";
pub const CLOSE_AGENT_TOOL: &str = "close_agent";

pub fn build_openai_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(apply_patch_tool());
    registry
}

pub fn build_anthropic_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(edit_file_tool());
    registry
}

pub fn build_gemini_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(edit_file_tool());
    registry
}

pub fn register_shared_core_tools(registry: &mut ToolRegistry) {
    registry.register(read_file_tool());
    registry.register(write_file_tool());
    registry.register(shell_tool());
    registry.register(grep_tool());
    registry.register(glob_tool());
}

pub fn register_subagent_tools(registry: &mut ToolRegistry) {
    registry.register(spawn_agent_tool());
    registry.register(send_input_tool());
    registry.register(wait_tool());
    registry.register(close_agent_tool());
}

fn read_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: READ_FILE_TOOL.to_string(),
            description: "Read a file from the filesystem. Returns line-numbered content."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path"],
                "properties": {
                    "file_path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let offset = optional_usize_argument(&args, "offset")?;
                let limit = optional_usize_argument(&args, "limit")?;

                let content = env.read_file(&file_path, offset, limit).await?;
                Ok(format_line_numbered_content(&content, offset.unwrap_or(1)))
            })
        }),
    }
}

fn write_file_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: WRITE_FILE_TOOL.to_string(),
            description:
                "Write content to a file. Creates the file and parent directories if needed."
                    .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["file_path", "content"],
                "properties": {
                    "file_path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let file_path = required_string_argument(&args, "file_path")?;
                let content = required_string_argument(&args, "content")?;
                env.write_file(&file_path, &content).await?;
                Ok(format!(
                    "Wrote {} bytes to {}",
                    content.as_bytes().len(),
                    file_path
                ))
            })
        }),
    }
}

fn shell_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SHELL_TOOL.to_string(),
            description: "Execute a shell command. Returns stdout, stderr, and exit code."
                .to_string(),
            parameters: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "description": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let command = required_string_argument(&args, "command")?;
                let timeout_ms = optional_u64_argument(&args, "timeout_ms")?.unwrap_or(0);
                let result = env.exec_command(&command, timeout_ms, None, None).await?;
                Ok(format_exec_result(&result))
            })
        }),
    }
}

fn grep_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: GREP_TOOL.to_string(),
            description: "Search file contents using regex patterns.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" },
                    "glob_filter": { "type": "string" },
                    "case_insensitive": { "type": "boolean" },
                    "max_results": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let pattern = required_string_argument(&args, "pattern")?;
                let path = optional_string_argument(&args, "path")?.unwrap_or(".".to_string());
                let options = GrepOptions {
                    glob_filter: optional_string_argument(&args, "glob_filter")?,
                    case_insensitive: optional_bool_argument(&args, "case_insensitive")?
                        .unwrap_or(false),
                    max_results: optional_usize_argument(&args, "max_results")?.or(Some(100)),
                };

                let output = env.grep(&pattern, &path, options).await?;
                if output.trim().is_empty() {
                    Ok("No matches found".to_string())
                } else {
                    Ok(output)
                }
            })
        }),
    }
}

fn glob_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: GLOB_TOOL.to_string(),
            description: "Find files matching a glob pattern.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["pattern"],
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: Arc::new(|args, env| {
            Box::pin(async move {
                let pattern = required_string_argument(&args, "pattern")?;
                let path = optional_string_argument(&args, "path")?.unwrap_or(".".to_string());
                let matches = env.glob(&pattern, &path).await?;
                if matches.is_empty() {
                    Ok("No files matched".to_string())
                } else {
                    Ok(matches.join("\n"))
                }
            })
        }),
    }
}

fn spawn_agent_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SPAWN_AGENT_TOOL.to_string(),
            description: "Spawn a subagent to handle a scoped task autonomously.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["task"],
                "properties": {
                    "task": { "type": "string" },
                    "working_dir": { "type": "string" },
                    "model": { "type": "string" },
                    "max_turns": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(SPAWN_AGENT_TOOL),
    }
}

fn send_input_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: SEND_INPUT_TOOL.to_string(),
            description: "Send a message to a running subagent.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id", "message"],
                "properties": {
                    "agent_id": { "type": "string" },
                    "message": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(SEND_INPUT_TOOL),
    }
}

fn wait_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: WAIT_TOOL.to_string(),
            description: "Wait for a subagent to complete and return its result.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id"],
                "properties": {
                    "agent_id": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(WAIT_TOOL),
    }
}

fn close_agent_tool() -> RegisteredTool {
    RegisteredTool {
        definition: ToolDefinition {
            name: CLOSE_AGENT_TOOL.to_string(),
            description: "Terminate a subagent.".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["agent_id"],
                "properties": {
                    "agent_id": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        executor: unsupported_subagent_executor(CLOSE_AGENT_TOOL),
    }
}

fn unsupported_subagent_executor(tool_name: &'static str) -> ToolExecutor {
    Arc::new(move |_args, _env| {
        Box::pin(async move {
            Err(ToolError::Execution(format!(
                "{} can only run inside a live Session dispatcher",
                tool_name
            ))
            .into())
        })
    })
}

fn edit_file_tool() -> RegisteredTool {
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
                let replacement_count = content.match_indices(&old_string).count();
                if replacement_count == 0 {
                    return Err(ToolError::Execution(format!(
                        "old_string not found in '{}'",
                        file_path
                    ))
                    .into());
                }
                if replacement_count > 1 && !replace_all {
                    return Err(ToolError::Execution(format!(
                        "old_string is not unique in '{}': found {} matches; provide more context or set replace_all=true",
                        file_path, replacement_count
                    ))
                    .into());
                }

                let next_content = if replace_all {
                    content.replace(&old_string, &new_string)
                } else {
                    content.replacen(&old_string, &new_string, 1)
                };
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

fn apply_patch_tool() -> RegisteredTool {
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
                let operations = parse_apply_patch(&patch)?;
                apply_patch_operations(&operations, env).await
            })
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PatchOperation {
    AddFile {
        path: String,
        lines: Vec<String>,
    },
    DeleteFile {
        path: String,
    },
    UpdateFile {
        path: String,
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PatchHunk {
    header: String,
    lines: Vec<PatchHunkLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PatchHunkLine {
    Context(String),
    Delete(String),
    Add(String),
    EndOfFile,
}

fn parse_apply_patch(patch: &str) -> Result<Vec<PatchOperation>, ToolError> {
    let lines: Vec<&str> = patch.lines().collect();
    if lines.first().copied() != Some("*** Begin Patch") {
        return Err(ToolError::Validation(
            "apply_patch payload must start with '*** Begin Patch'".to_string(),
        ));
    }
    if lines.last().copied() != Some("*** End Patch") {
        return Err(ToolError::Validation(
            "apply_patch payload must end with '*** End Patch'".to_string(),
        ));
    }

    let mut operations = Vec::new();
    let mut idx = 1usize;
    let end = lines.len().saturating_sub(1);
    while idx < end {
        let line = lines[idx];
        if line.trim().is_empty() {
            idx += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            idx += 1;
            let mut added = Vec::new();
            while idx < end && !is_patch_operation_start(lines[idx]) {
                let Some(payload) = lines[idx].strip_prefix('+') else {
                    return Err(ToolError::Validation(format!(
                        "invalid add-file line: '{}'",
                        lines[idx]
                    )));
                };
                added.push(payload.to_string());
                idx += 1;
            }
            operations.push(PatchOperation::AddFile {
                path: path.to_string(),
                lines: added,
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(PatchOperation::DeleteFile {
                path: path.to_string(),
            });
            idx += 1;
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Update File: ") {
            idx += 1;
            let mut move_to = None;
            if idx < end {
                if let Some(target) = lines[idx].strip_prefix("*** Move to: ") {
                    move_to = Some(target.to_string());
                    idx += 1;
                }
            }

            let mut hunks = Vec::new();
            while idx < end && !is_patch_operation_start(lines[idx]) {
                let header = lines[idx];
                if !header.starts_with("@@") {
                    return Err(ToolError::Validation(format!(
                        "invalid hunk header in update '{}': '{}'",
                        path, header
                    )));
                }
                idx += 1;

                let mut hunk_lines = Vec::new();
                while idx < end
                    && !is_patch_operation_start(lines[idx])
                    && !lines[idx].starts_with("@@")
                {
                    let hunk_line = lines[idx];
                    if hunk_line == "*** End of File" {
                        hunk_lines.push(PatchHunkLine::EndOfFile);
                        idx += 1;
                        continue;
                    }
                    let Some(prefix) = hunk_line.chars().next() else {
                        return Err(ToolError::Validation(
                            "empty hunk line is not allowed".to_string(),
                        ));
                    };
                    let value = hunk_line[1..].to_string();
                    let parsed = match prefix {
                        ' ' => PatchHunkLine::Context(value),
                        '-' => PatchHunkLine::Delete(value),
                        '+' => PatchHunkLine::Add(value),
                        _ => {
                            return Err(ToolError::Validation(format!(
                                "invalid hunk line prefix '{}' in '{}'",
                                prefix, hunk_line
                            )));
                        }
                    };
                    hunk_lines.push(parsed);
                    idx += 1;
                }

                if hunk_lines.is_empty() {
                    return Err(ToolError::Validation(format!(
                        "empty hunk in update '{}'",
                        path
                    )));
                }
                hunks.push(PatchHunk {
                    header: header.to_string(),
                    lines: hunk_lines,
                });
            }

            if hunks.is_empty() {
                return Err(ToolError::Validation(format!(
                    "update operation for '{}' must include at least one hunk",
                    path
                )));
            }

            operations.push(PatchOperation::UpdateFile {
                path: path.to_string(),
                move_to,
                hunks,
            });
            continue;
        }

        return Err(ToolError::Validation(format!(
            "unknown patch operation line: '{}'",
            line
        )));
    }

    if operations.is_empty() {
        return Err(ToolError::Validation(
            "patch must contain at least one operation".to_string(),
        ));
    }

    Ok(operations)
}

fn is_patch_operation_start(line: &str) -> bool {
    line.starts_with("*** Add File: ")
        || line.starts_with("*** Delete File: ")
        || line.starts_with("*** Update File: ")
}

async fn apply_patch_operations(
    operations: &[PatchOperation],
    env: Arc<dyn ExecutionEnvironment>,
) -> Result<String, AgentError> {
    let mut summaries = Vec::new();
    for operation in operations {
        match operation {
            PatchOperation::AddFile { path, lines } => {
                if env.file_exists(path).await? {
                    return Err(
                        ToolError::Execution(format!("file already exists: '{}'", path)).into(),
                    );
                }
                env.write_file(path, &lines.join("\n")).await?;
                summaries.push(format!("A {}", path));
            }
            PatchOperation::DeleteFile { path } => {
                if !env.file_exists(path).await? {
                    return Err(ToolError::Execution(format!("file not found: '{}'", path)).into());
                }
                env.delete_file(path).await?;
                summaries.push(format!("D {}", path));
            }
            PatchOperation::UpdateFile {
                path,
                move_to,
                hunks,
            } => {
                if !env.file_exists(path).await? {
                    return Err(ToolError::Execution(format!(
                        "cannot update missing file '{}'",
                        path
                    ))
                    .into());
                }

                let original = env.read_file(path, None, None).await?;
                let updated = apply_hunks_to_content(&original, hunks).map_err(AgentError::from)?;

                let move_target = move_to.as_deref().filter(|target| *target != path.as_str());
                if let Some(target_path) = move_target {
                    if env.file_exists(target_path).await? {
                        return Err(ToolError::Execution(format!(
                            "move target already exists: '{}'",
                            target_path
                        ))
                        .into());
                    }
                    env.write_file(path, &updated).await?;
                    env.move_file(path, target_path).await?;
                    summaries.push(format!("R {} -> {}", path, target_path));
                } else {
                    env.write_file(path, &updated).await?;
                    summaries.push(format!("M {}", path));
                }
            }
        }
    }

    Ok(format!("Applied patch:\n{}", summaries.join("\n")))
}

fn apply_hunks_to_content(content: &str, hunks: &[PatchHunk]) -> Result<String, ToolError> {
    let mut lines = split_content_lines(content);
    let had_trailing_newline = content.ends_with('\n');
    let mut search_from = 0usize;

    for hunk in hunks {
        let (old_lines, new_lines) = hunk_old_new_lines(hunk);
        if old_lines.is_empty() {
            let insert_at = search_from.min(lines.len());
            lines.splice(insert_at..insert_at, new_lines.clone());
            search_from = insert_at + new_lines.len();
            continue;
        }

        let position = find_subsequence(&lines, &old_lines, search_from)
            .or_else(|| find_subsequence(&lines, &old_lines, 0))
            .ok_or_else(|| {
                ToolError::Execution(format!("failed to match hunk '{}'", hunk.header))
            })?;
        let end = position + old_lines.len();
        lines.splice(position..end, new_lines.clone());
        search_from = position + new_lines.len();
    }

    let mut updated = lines.join("\n");
    if had_trailing_newline {
        updated.push('\n');
    }
    Ok(updated)
}

fn split_content_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<String> = content.split('\n').map(str::to_string).collect();
    if content.ends_with('\n') && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn hunk_old_new_lines(hunk: &PatchHunk) -> (Vec<String>, Vec<String>) {
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    for line in &hunk.lines {
        match line {
            PatchHunkLine::Context(value) => {
                old_lines.push(value.clone());
                new_lines.push(value.clone());
            }
            PatchHunkLine::Delete(value) => old_lines.push(value.clone()),
            PatchHunkLine::Add(value) => new_lines.push(value.clone()),
            PatchHunkLine::EndOfFile => {}
        }
    }
    (old_lines, new_lines)
}

fn find_subsequence(haystack: &[String], needle: &[String], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(start.min(haystack.len()));
    }
    if start >= haystack.len() || needle.len() > haystack.len() {
        return None;
    }

    let limit = haystack.len().saturating_sub(needle.len());
    for idx in start..=limit {
        if haystack[idx..idx + needle.len()] == *needle {
            return Some(idx);
        }
    }
    None
}

fn required_string_argument(arguments: &Value, key: &str) -> Result<String, ToolError> {
    optional_string_argument(arguments, key)?
        .ok_or_else(|| ToolError::Validation(format!("missing required argument '{}'", key)))
}

fn optional_string_argument(arguments: &Value, key: &str) -> Result<Option<String>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a string",
            key
        )));
    };
    Ok(Some(value.to_string()))
}

fn optional_bool_argument(arguments: &Value, key: &str) -> Result<Option<bool>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_bool() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a boolean",
            key
        )));
    };
    Ok(Some(value))
}

fn optional_u64_argument(arguments: &Value, key: &str) -> Result<Option<u64>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a positive integer",
            key
        )));
    };
    Ok(Some(value))
}

fn optional_usize_argument(arguments: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    Ok(optional_u64_argument(arguments, key)?.map(|value| value as usize))
}

fn format_line_numbered_content(content: &str, start_line: usize) -> String {
    if content.is_empty() {
        return String::new();
    }
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("{} | {}", start_line + idx, line))
        .collect::<Vec<String>>()
        .join("\n")
}

fn format_exec_result(result: &crate::ExecResult) -> String {
    let mut output = format!(
        "exit_code: {}\nduration_ms: {}",
        result.exit_code, result.duration_ms
    );
    if !result.stdout.is_empty() {
        output.push_str("\nstdout:\n");
        output.push_str(&result.stdout);
    }
    if !result.stderr.is_empty() {
        output.push_str("\nstderr:\n");
        output.push_str(&result.stderr);
    }
    output
}

fn tool_error_result(tool_call_id: String, message: String) -> ToolResult {
    ToolResult {
        tool_call_id,
        content: Value::String(message),
        is_error: true,
    }
}

fn parse_tool_arguments(tool_call: &ToolCall) -> Result<Value, ToolError> {
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        let parsed = serde_json::from_str::<Value>(raw_arguments).map_err(|error| {
            ToolError::Validation(format!(
                "invalid JSON arguments for tool '{}': {}",
                tool_call.name, error
            ))
        })?;
        return Ok(parsed);
    }

    Ok(tool_call.arguments.clone())
}

fn validate_tool_arguments(schema: &Value, arguments: &Value) -> Result<(), ToolError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| ToolError::Validation("tool arguments must be a JSON object".to_string()))?;

    let schema_object = schema.as_object().ok_or_else(|| {
        ToolError::Validation("tool schema root must be a JSON object".to_string())
    })?;

    if schema_object
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|schema_type| schema_type != "object")
    {
        return Err(ToolError::Validation(
            "tool schema root type must be 'object'".to_string(),
        ));
    }

    if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(ToolError::Validation(format!(
                    "missing required argument '{}'",
                    key
                )));
            }
        }
    }

    let properties = schema_object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let additional_allowed = schema_object
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    for (key, value) in object {
        let Some(property) = properties.get(key) else {
            if additional_allowed {
                continue;
            }
            return Err(ToolError::Validation(format!(
                "unexpected argument '{}' not allowed by schema",
                key
            )));
        };

        if let Some(type_name) = property.get("type").and_then(Value::as_str) {
            let is_valid = match type_name {
                "string" => value.is_string(),
                "number" => value.is_number(),
                "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
                "boolean" => value.is_boolean(),
                "array" => value.is_array(),
                "object" => value.is_object(),
                "null" => value.is_null(),
                _ => true,
            };

            if !is_valid {
                return Err(ToolError::Validation(format!(
                    "argument '{}' expected type '{}' but received '{}'",
                    key,
                    type_name,
                    json_type_name(value)
                )));
            }
        }
    }

    Ok(())
}

fn json_type_name(value: &Value) -> &'static str {
    if value.is_null() {
        "null"
    } else if value.is_boolean() {
        "boolean"
    } else if value.is_string() {
        "string"
    } else if value.is_number() {
        "number"
    } else if value.is_array() {
        "array"
    } else {
        "object"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BufferedEventEmitter, EventKind, LocalExecutionEnvironment, NoopEventEmitter};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::time::{Duration, Instant, sleep};

    fn dummy_executor() -> ToolExecutor {
        Arc::new(|_args, _env| Box::pin(async move { Ok("ok".to_string()) }))
    }

    #[test]
    fn tool_registry_latest_registration_wins() {
        let mut registry = ToolRegistry::default();

        let first = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "first".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(first);

        let second = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "second".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(second);

        let registered = registry
            .get("read_file")
            .expect("tool should be present after replacement");
        assert_eq!(registered.definition.description, "second");
    }

    #[test]
    fn tool_registry_definitions_are_sorted_by_name() {
        let mut registry = ToolRegistry::default();
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "zeta".to_string(),
                description: "z".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        });
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "alpha".to_string(),
                description: "a".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        });

        let names: Vec<String> = registry
            .definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(names, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    struct TestExecutionEnvironment {
        working_dir: PathBuf,
    }

    impl Default for TestExecutionEnvironment {
        fn default() -> Self {
            Self {
                working_dir: PathBuf::from("."),
            }
        }
    }

    struct TimeoutCaptureEnv {
        working_dir: PathBuf,
        observed_timeout_ms: Arc<AtomicU64>,
    }

    impl TimeoutCaptureEnv {
        fn new(observed_timeout_ms: Arc<AtomicU64>) -> Self {
            Self {
                working_dir: PathBuf::from("."),
                observed_timeout_ms,
            }
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for TimeoutCaptureEnv {
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
            timeout_ms: u64,
            _working_dir: Option<&str>,
            _env_vars: Option<HashMap<String, String>>,
        ) -> Result<crate::ExecResult, AgentError> {
            self.observed_timeout_ms.store(timeout_ms, Ordering::SeqCst);
            Ok(crate::ExecResult {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
                timed_out: false,
                duration_ms: 1,
            })
        }

        async fn grep(
            &self,
            _pattern: &str,
            _path: &str,
            _options: crate::GrepOptions,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("grep".to_string()))
        }

        async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
            Err(AgentError::NotImplemented("glob".to_string()))
        }

        fn working_directory(&self) -> &Path {
            &self.working_dir
        }

        fn platform(&self) -> &str {
            "linux"
        }

        fn os_version(&self) -> &str {
            "test"
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for TestExecutionEnvironment {
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
            _options: crate::GrepOptions,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("grep".to_string()))
        }

        async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
            Err(AgentError::NotImplemented("glob".to_string()))
        }

        fn working_directory(&self) -> &Path {
            &self.working_dir
        }

        fn platform(&self) -> &str {
            "linux"
        }

        fn os_version(&self) -> &str {
            "test"
        }
    }

    fn command_tool(executor: ToolExecutor) -> RegisteredTool {
        RegisteredTool {
            definition: ToolDefinition {
                name: "shell".to_string(),
                description: "run command".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["command"],
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            executor,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_unknown_tool_returns_error_result_instead_of_failing_session() {
        let registry = ToolRegistry::default();
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "unknown".to_string(),
                    arguments: serde_json::json!({}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(results[0].tool_call_id, "call-1");
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("Unknown tool")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_validation_error_returns_structured_tool_error_without_execution() {
        let execution_count = Arc::new(AtomicUsize::new(0));
        let count = execution_count.clone();
        let executor: ToolExecutor = Arc::new(move |_args, _env| {
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok("should not run".to_string())
            })
        });

        let mut registry = ToolRegistry::default();
        registry.register(command_tool(executor));

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({ "not_command": 1 }),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(execution_count.load(Ordering::SeqCst), 0);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("missing required argument 'command'")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_parses_raw_json_arguments_and_validates_schema() {
        let executor: ToolExecutor = Arc::new(move |args, _env| {
            Box::pin(async move {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(format!("ran {cmd}"))
            })
        });
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(executor));

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({}),
                    raw_arguments: Some("{\"command\":\"echo hi\"}".to_string()),
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(!results[0].is_error);
        assert_eq!(results[0].content.as_str(), Some("ran echo hi"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_parallel_mode_keeps_input_order_and_call_ids_stable() {
        let executor: ToolExecutor = Arc::new(move |args, _env| {
            Box::pin(async move {
                let delay_ms = args
                    .get("delay_ms")
                    .and_then(Value::as_u64)
                    .expect("delay_ms should be present");
                let output = args
                    .get("output")
                    .and_then(Value::as_str)
                    .expect("output should be present")
                    .to_string();
                sleep(Duration::from_millis(delay_ms)).await;
                Ok(output)
            })
        });

        let mut registry = ToolRegistry::default();
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "sleep_echo".to_string(),
                description: "sleep and echo".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["delay_ms", "output"],
                    "properties": {
                        "delay_ms": { "type": "integer" },
                        "output": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            executor,
        });

        let calls = vec![
            ToolCall {
                id: "call-a".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 80, "output": "a"}),
                raw_arguments: None,
            },
            ToolCall {
                id: "call-b".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 20, "output": "b"}),
                raw_arguments: None,
            },
            ToolCall {
                id: "call-c".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 60, "output": "c"}),
                raw_arguments: None,
            },
        ];

        let started = Instant::now();
        let results = registry
            .dispatch(
                calls,
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: true,
                },
            )
            .await
            .expect("dispatch should not fail");
        let elapsed = started.elapsed();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].tool_call_id, "call-a");
        assert_eq!(results[1].tool_call_id, "call-b");
        assert_eq!(results[2].tool_call_id, "call-c");
        assert_eq!(results[0].content.as_str(), Some("a"));
        assert_eq!(results[1].content.as_str(), Some("b"));
        assert_eq!(results[2].content.as_str(), Some("c"));
        assert!(elapsed < Duration::from_millis(170));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_emits_tool_call_start_and_end_events_in_order() {
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(Arc::new(|_args, _env| {
            Box::pin(async move { Ok("done".to_string()) })
        })));

        let emitter = Arc::new(BufferedEventEmitter::default());
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                emitter.clone(),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let events = emitter.snapshot();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, EventKind::ToolCallStart);
        assert_eq!(events[1].kind, EventKind::ToolCallOutputDelta);
        assert_eq!(events[2].kind, EventKind::ToolCallEnd);
        assert_eq!(events[0].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("delta"), Some("done"));
        assert_eq!(events[2].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[2].data.get_str("output"), Some("done"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_returns_truncated_result_to_llm_but_emits_full_output_event() {
        let full_output = "x".repeat(40_000);
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(Arc::new(move |_args, _env| {
            let full_output = full_output.clone();
            Box::pin(async move { Ok(full_output) })
        })));

        let emitter = Arc::new(BufferedEventEmitter::default());
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                emitter.clone(),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        let llm_output = results[0]
            .content
            .as_str()
            .expect("output should be a string");
        assert!(llm_output.contains("[WARNING: Tool output was truncated."));
        assert!(llm_output.chars().count() < 40_000);

        let events = emitter.snapshot();
        let end_event = events
            .iter()
            .find(|event| event.kind == EventKind::ToolCallEnd)
            .expect("tool end event should be present");
        let event_output = end_event
            .data
            .get_str("output")
            .expect("output field should be present");
        assert_eq!(event_output.chars().count(), 40_000);
        assert!(event_output.chars().all(|ch| ch == 'x'));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_dispatch_injects_default_timeout_from_session_config() {
        let observed_timeout = Arc::new(AtomicU64::new(0));
        let env = Arc::new(TimeoutCaptureEnv::new(observed_timeout.clone()));
        let mut registry = ToolRegistry::default();
        registry.register(shell_tool());

        let mut config = SessionConfig::default();
        config.default_command_timeout_ms = 12_345;
        config.max_command_timeout_ms = 60_000;

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "echo hi" }),
                    raw_arguments: None,
                }],
                env,
                &config,
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        assert_eq!(observed_timeout.load(Ordering::SeqCst), 12_345);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_dispatch_clamps_timeout_to_session_max() {
        let observed_timeout = Arc::new(AtomicU64::new(0));
        let env = Arc::new(TimeoutCaptureEnv::new(observed_timeout.clone()));
        let mut registry = ToolRegistry::default();
        registry.register(shell_tool());

        let mut config = SessionConfig::default();
        config.default_command_timeout_ms = 1_000;
        config.max_command_timeout_ms = 1_500;

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "echo hi", "timeout_ms": 30_000 }),
                    raw_arguments: None,
                }],
                env,
                &config,
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        assert_eq!(observed_timeout.load(Ordering::SeqCst), 1_500);
    }

    #[test]
    fn build_openai_registry_uses_apply_patch_variant() {
        let openai = build_openai_tool_registry();
        let anthropic = build_anthropic_tool_registry();
        let gemini = build_gemini_tool_registry();

        assert!(openai.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(!openai.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(anthropic.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(!anthropic.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(gemini.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(!gemini.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(openai.names().contains(&SPAWN_AGENT_TOOL.to_string()));
        assert!(openai.names().contains(&SEND_INPUT_TOOL.to_string()));
        assert!(openai.names().contains(&WAIT_TOOL.to_string()));
        assert!(openai.names().contains(&CLOSE_AGENT_TOOL.to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_file_returns_ambiguity_error_when_match_is_not_unique() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("target.txt", "alpha\nalpha\n")
            .await
            .expect("seed file should write");

        let registry = build_anthropic_tool_registry();
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: EDIT_FILE_TOOL.to_string(),
                    arguments: json!({
                        "file_path": "target.txt",
                        "old_string": "alpha",
                        "new_string": "beta"
                    }),
                    raw_arguments: None,
                }],
                env,
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(results[0].is_error);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("not unique")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_patch_returns_parse_error_for_invalid_hunk_header() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("target.txt", "one\n")
            .await
            .expect("seed file should write");

        let registry = build_openai_tool_registry();
        let patch = "*** Begin Patch\n*** Update File: target.txt\nnot-a-hunk\n*** End Patch";
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: APPLY_PATCH_TOOL.to_string(),
                    arguments: json!({
                        "patch": patch
                    }),
                    raw_arguments: None,
                }],
                env,
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(results[0].is_error);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("invalid hunk header")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_patch_supports_successful_multi_file_operations() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("a.txt", "line1\nline2\n")
            .await
            .expect("seed a.txt");
        env.write_file("old_name.txt", "use old_dep;\n")
            .await
            .expect("seed old_name");
        env.write_file("delete_me.txt", "bye\n")
            .await
            .expect("seed delete_me");

        let registry = build_openai_tool_registry();
        let patch = "\
*** Begin Patch
*** Add File: new_file.txt
+alpha
+beta
*** Update File: a.txt
@@ replace line
 line1
-line2
+line-two
*** Update File: old_name.txt
*** Move to: new_name.txt
@@ rename import
-use old_dep;
+use new_dep;
*** Delete File: delete_me.txt
*** End Patch";

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: APPLY_PATCH_TOOL.to_string(),
                    arguments: json!({
                        "patch": patch
                    }),
                    raw_arguments: None,
                }],
                env.clone(),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let summary = results[0].content.as_str().unwrap_or_default();
        assert!(summary.contains("A new_file.txt"));
        assert!(summary.contains("M a.txt"));
        assert!(summary.contains("R old_name.txt -> new_name.txt"));
        assert!(summary.contains("D delete_me.txt"));

        let updated_a = env
            .read_file("a.txt", None, None)
            .await
            .expect("updated a.txt should read");
        assert_eq!(updated_a, "line1\nline-two\n");

        let new_file = env
            .read_file("new_file.txt", None, None)
            .await
            .expect("new file should read");
        assert_eq!(new_file, "alpha\nbeta");

        let renamed = env
            .read_file("new_name.txt", None, None)
            .await
            .expect("renamed file should read");
        assert_eq!(renamed, "use new_dep;\n");

        assert!(
            !env.file_exists("old_name.txt")
                .await
                .expect("old name existence should be checked")
        );
        assert!(
            !env.file_exists("delete_me.txt")
                .await
                .expect("delete target existence should be checked")
        );
    }
}
