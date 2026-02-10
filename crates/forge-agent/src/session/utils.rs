use super::{
    AgentError, EnvironmentContext, ExecutionEnvironment, Message, ProjectDocument,
    ProviderProfile, Session, SessionError, SubAgentResult, SubAgentStatus, SubAgentTaskOutput,
    ToolCall, ToolError, Turn,
};
use forge_llm::{ContentPart, Role, ThinkingData, ToolCallData};
use serde_json::Value;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn is_subagent_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "spawn_agent" | "send_input" | "wait" | "close_agent"
    )
}

pub(super) fn parse_tool_call_arguments(tool_call: &ToolCall) -> Result<Value, AgentError> {
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

pub(super) fn required_string_argument(arguments: &Value, key: &str) -> Result<String, AgentError> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Validation(format!("missing required argument '{}'", key)))?;
    Ok(value.to_string())
}

pub(super) fn optional_string_argument(
    arguments: &Value,
    key: &str,
) -> Result<Option<String>, AgentError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(ToolError::Validation(format!("argument '{}' must be a string", key)).into());
    };
    Ok(Some(value.to_string()))
}

pub(super) fn optional_usize_argument(
    arguments: &Value,
    key: &str,
) -> Result<Option<usize>, AgentError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(ToolError::Validation(format!("argument '{}' must be an integer", key)).into());
    };
    Ok(Some(value as usize))
}

pub(super) fn latest_assistant_output(history: &[Turn]) -> Option<String> {
    history.iter().rev().find_map(|turn| {
        if let Turn::Assistant(assistant) = turn {
            Some(assistant.content.clone())
        } else {
            None
        }
    })
}

pub(super) fn spawn_subagent_submit_task(
    mut session: Box<Session>,
    input: String,
) -> tokio::task::JoinHandle<SubAgentTaskOutput> {
    tokio::spawn(async move {
        let completion = session.submit(input).await;
        let result = match completion {
            Ok(_) => SubAgentResult {
                output: latest_assistant_output(session.history()).unwrap_or_default(),
                success: true,
                turns_used: session.history().len(),
            },
            Err(error) => SubAgentResult {
                output: error.to_string(),
                success: false,
                turns_used: session.history().len(),
            },
        };
        SubAgentTaskOutput { session, result }
    })
}

pub(super) fn resolve_subagent_working_directory(
    parent_working_directory: &Path,
    requested: &str,
) -> Result<PathBuf, AgentError> {
    let requested_path = Path::new(requested);
    let candidate = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        parent_working_directory.join(requested_path)
    };

    let canonical = canonicalize_or_fallback(&candidate);
    if !canonical.exists() || !canonical.is_dir() {
        return Err(ToolError::Execution(format!(
            "subagent working_dir '{}' does not exist or is not a directory",
            requested
        ))
        .into());
    }

    Ok(canonical)
}

pub(super) fn subagent_status_label(status: &SubAgentStatus) -> &'static str {
    match status {
        SubAgentStatus::Running => "running",
        SubAgentStatus::Completed => "completed",
        SubAgentStatus::Failed => "failed",
    }
}

pub(super) fn should_transition_to_awaiting_input(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.ends_with('?') {
        return false;
    }

    let word_count = trimmed
        .split_whitespace()
        .filter(|segment| segment.chars().any(char::is_alphabetic))
        .count();
    word_count >= 3
}

pub(super) fn convert_history_to_messages(history: &[Turn]) -> Vec<Message> {
    let mut messages = Vec::new();

    for turn in history {
        match turn {
            Turn::User(turn) => messages.push(Message::user(turn.content.clone())),
            Turn::Assistant(turn) => {
                let mut content = Vec::new();
                if !turn.content.is_empty() {
                    content.push(ContentPart::text(turn.content.clone()));
                }

                if let Some(reasoning) = &turn.reasoning {
                    if !reasoning.is_empty() {
                        content.push(ContentPart::thinking(ThinkingData {
                            text: reasoning.clone(),
                            signature: None,
                            redacted: false,
                        }));
                    }
                }

                for tool_call in &turn.tool_calls {
                    content.push(ContentPart::tool_call(ToolCallData {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                        r#type: "function".to_string(),
                    }));
                }

                if content.is_empty() {
                    content.push(ContentPart::text(String::new()));
                }

                messages.push(Message {
                    role: Role::Assistant,
                    content,
                    name: None,
                    tool_call_id: None,
                });
            }
            Turn::ToolResults(turn) => {
                for result in &turn.results {
                    messages.push(Message::tool_result(
                        result.tool_call_id.clone(),
                        result.content.clone(),
                        result.is_error,
                    ));
                }
            }
            Turn::System(turn) => messages.push(Message::system(turn.content.clone())),
            Turn::Steering(turn) => messages.push(Message::user(turn.content.clone())),
        }
    }

    messages
}

pub(super) fn current_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

pub(super) fn current_date_yyyy_mm_dd() -> String {
    #[cfg(windows)]
    let command = ("cmd", vec!["/C", "echo %date%"]);
    #[cfg(not(windows))]
    let command = ("date", vec!["+%Y-%m-%d"]);

    let output = std::process::Command::new(command.0)
        .args(command.1)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    "1970-01-01".to_string()
}

pub(super) fn build_environment_context_snapshot(
    provider_profile: &dyn ProviderProfile,
    execution_env: &dyn ExecutionEnvironment,
) -> EnvironmentContext {
    let working_directory = canonicalize_or_fallback(execution_env.working_directory());
    let repository_root = find_git_repository_root(&working_directory);
    let (git_branch, git_status_summary, git_recent_commits) = if let Some(root) = &repository_root
    {
        (
            git_current_branch(root),
            git_status_summary(root),
            git_recent_commits(root, 5),
        )
    } else {
        (None, None, Vec::new())
    };

    EnvironmentContext {
        working_directory: working_directory.to_string_lossy().to_string(),
        repository_root: repository_root
            .as_ref()
            .map(|root| root.to_string_lossy().to_string()),
        platform: execution_env.platform().to_string(),
        os_version: execution_env.os_version().to_string(),
        is_git_repository: repository_root.is_some(),
        git_branch,
        git_status_summary,
        git_recent_commits,
        date_yyyy_mm_dd: current_date_yyyy_mm_dd(),
        model: provider_profile.model().to_string(),
        knowledge_cutoff: provider_profile.knowledge_cutoff().map(str::to_string),
    }
}

pub(super) fn discover_project_documents(
    working_directory: &Path,
    provider_profile: &dyn ProviderProfile,
) -> Vec<ProjectDocument> {
    const PROJECT_DOC_BYTE_BUDGET: usize = 32 * 1024;
    let working_directory = canonicalize_or_fallback(working_directory);
    let root =
        find_git_repository_root(&working_directory).unwrap_or_else(|| working_directory.clone());
    let directories = path_chain_from_root_to_cwd(&root, &working_directory);
    let instruction_files = provider_profile.project_instruction_files();

    let mut docs = Vec::new();
    for directory in directories {
        for instruction_file in &instruction_files {
            let candidate = directory.join(instruction_file);
            if !candidate.is_file() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&candidate) else {
                continue;
            };
            let relative = candidate
                .strip_prefix(&root)
                .unwrap_or(&candidate)
                .to_string_lossy()
                .replace('\\', "/");
            docs.push(ProjectDocument {
                path: relative,
                content,
            });
        }
    }

    truncate_project_documents_to_budget(docs, PROJECT_DOC_BYTE_BUDGET)
}

pub(super) fn truncate_project_documents_to_budget(
    docs: Vec<ProjectDocument>,
    byte_budget: usize,
) -> Vec<ProjectDocument> {
    let total_bytes: usize = docs
        .iter()
        .map(|document| document.content.as_bytes().len())
        .sum();
    if total_bytes <= byte_budget {
        return docs;
    }

    let mut used = 0usize;
    let mut truncated_docs = Vec::new();
    for document in docs {
        if used >= byte_budget {
            break;
        }

        let document_bytes = document.content.as_bytes().len();
        if used + document_bytes <= byte_budget {
            used += document_bytes;
            truncated_docs.push(document);
            continue;
        }

        let remaining = byte_budget.saturating_sub(used);
        let visible = truncate_str_to_byte_limit(&document.content, remaining);
        let content = if visible.is_empty() {
            crate::profiles::PROJECT_DOC_TRUNCATION_MARKER.to_string()
        } else {
            format!(
                "{}\n{}",
                visible,
                crate::profiles::PROJECT_DOC_TRUNCATION_MARKER
            )
        };
        truncated_docs.push(ProjectDocument {
            path: document.path,
            content,
        });
        break;
    }

    truncated_docs
}

pub(super) fn truncate_str_to_byte_limit(input: &str, max_bytes: usize) -> String {
    if input.as_bytes().len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    let mut end = max_bytes.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

pub(super) fn find_git_repository_root(start: &Path) -> Option<PathBuf> {
    let canonical = canonicalize_or_fallback(start);
    for ancestor in canonical.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub(super) fn path_chain_from_root_to_cwd(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let root = canonicalize_or_fallback(root);
    let cwd = canonicalize_or_fallback(cwd);
    if root == cwd {
        return vec![cwd];
    }
    if !cwd.starts_with(&root) {
        return vec![cwd];
    }

    let mut chain = Vec::new();
    let mut current = cwd.as_path();
    loop {
        chain.push(current.to_path_buf());
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            return vec![cwd];
        };
        current = parent;
    }
    chain.reverse();
    chain
}

pub(super) fn canonicalize_or_fallback(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn git_current_branch(repository_root: &Path) -> Option<String> {
    run_git_command(repository_root, &["rev-parse", "--abbrev-ref", "HEAD"])
}

pub(super) fn git_status_summary(repository_root: &Path) -> Option<String> {
    let output = run_git_command(repository_root, &["status", "--porcelain"])?;
    let mut modified = 0usize;
    let mut untracked = 0usize;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with("??") {
            untracked += 1;
        } else {
            modified += 1;
        }
    }
    Some(format!("modified: {modified}, untracked: {untracked}"))
}

pub(super) fn git_recent_commits(repository_root: &Path, limit: usize) -> Vec<String> {
    run_git_command(
        repository_root,
        &["log", "--oneline", "-n", &limit.to_string()],
    )
    .map(|output| {
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

pub(super) fn run_git_command(repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Some(String::new());
    }
    Some(text)
}

pub(super) fn validate_reasoning_effort(value: &str) -> Result<(), AgentError> {
    let normalized = value.to_ascii_lowercase();
    match normalized.as_str() {
        "low" | "medium" | "high" => Ok(()),
        _ => Err(SessionError::InvalidConfiguration(format!(
            "reasoning_effort must be one of: low, medium, high (received '{}')",
            value
        ))
        .into()),
    }
}

pub(super) fn detect_loop(history: &[Turn], window_size: usize) -> bool {
    if window_size == 0 {
        return false;
    }

    let signatures: Vec<u64> = history
        .iter()
        .filter_map(|turn| {
            if let Turn::Assistant(turn) = turn {
                Some(
                    turn.tool_calls
                        .iter()
                        .map(tool_call_signature)
                        .collect::<Vec<u64>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect();

    if signatures.len() < window_size {
        return false;
    }

    let recent = &signatures[signatures.len() - window_size..];
    for pattern_len in 1..=3 {
        if window_size % pattern_len != 0 {
            continue;
        }

        let pattern = &recent[0..pattern_len];
        let mut all_match = true;
        for chunk in recent.chunks(pattern_len).skip(1) {
            if chunk != pattern {
                all_match = false;
                break;
            }
        }
        if all_match {
            return true;
        }
    }

    false
}

pub(super) fn tool_call_signature(tool_call: &forge_llm::ToolCall) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_call.name.hash(&mut hasher);
    if let Ok(serialized) = serde_json::to_string(&tool_call.arguments) {
        serialized.hash(&mut hasher);
    } else {
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        raw_arguments.hash(&mut hasher);
    }
    hasher.finish()
}

pub(super) fn approximate_context_tokens(history: &[Turn]) -> usize {
    total_chars_in_history(history) / 4
}

pub(super) fn total_chars_in_history(history: &[Turn]) -> usize {
    history
        .iter()
        .map(|turn| match turn {
            Turn::User(turn) => turn.content.chars().count(),
            Turn::Assistant(turn) => {
                let mut chars = turn.content.chars().count();
                if let Some(reasoning) = &turn.reasoning {
                    chars += reasoning.chars().count();
                }
                for tool_call in &turn.tool_calls {
                    chars += tool_call.id.chars().count();
                    chars += tool_call.name.chars().count();
                    chars += tool_call.arguments.to_string().chars().count();
                    if let Some(raw) = &tool_call.raw_arguments {
                        chars += raw.chars().count();
                    }
                }
                chars
            }
            Turn::ToolResults(turn) => turn
                .results
                .iter()
                .map(|result| {
                    result.tool_call_id.chars().count() + result.content.to_string().chars().count()
                })
                .sum(),
            Turn::System(turn) => turn.content.chars().count(),
            Turn::Steering(turn) => turn.content.chars().count(),
        })
        .sum()
}
