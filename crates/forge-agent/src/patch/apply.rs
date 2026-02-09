use std::sync::Arc;

use crate::{AgentError, ExecutionEnvironment, ToolError};

use super::matching::{find_subsequence, find_subsequence_fuzzy_unique};
use super::types::{PatchHunk, PatchHunkLine, PatchOperation};

pub(crate) async fn apply_patch_operations(
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

        let position = if let Some(index) = find_subsequence(&lines, &old_lines, search_from)
            .or_else(|| find_subsequence(&lines, &old_lines, 0))
        {
            index
        } else {
            match find_subsequence_fuzzy_unique(&lines, &old_lines, search_from) {
                Ok(Some(index)) => index,
                Ok(None) => {
                    return Err(ToolError::Execution(format!(
                        "failed to match hunk '{}' (exact and fuzzy matching failed)",
                        hunk.header
                    )));
                }
                Err(matches) => {
                    return Err(ToolError::Execution(format!(
                        "failed to match hunk '{}': fuzzy match is ambiguous ({} candidates)",
                        hunk.header, matches
                    )));
                }
            }
        };

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

#[cfg(test)]
mod tests {
    use super::apply_hunks_to_content;
    use crate::patch::types::{PatchHunk, PatchHunkLine};

    #[test]
    fn apply_hunks_to_content_exact_match_updates_content() {
        let hunks = vec![PatchHunk {
            header: "@@ update".to_string(),
            lines: vec![
                PatchHunkLine::Delete("line2".to_string()),
                PatchHunkLine::Add("line-two".to_string()),
            ],
        }];
        let updated = apply_hunks_to_content("line1\nline2\n", &hunks).expect("should apply");
        assert_eq!(updated, "line1\nline-two\n");
    }

    #[test]
    fn apply_hunks_to_content_fuzzy_match_handles_whitespace_difference() {
        let hunks = vec![PatchHunk {
            header: "@@ update".to_string(),
            lines: vec![
                PatchHunkLine::Delete("fn greet() {".to_string()),
                PatchHunkLine::Delete("println!(\"hi\");".to_string()),
                PatchHunkLine::Add("fn greet() {".to_string()),
                PatchHunkLine::Add("println!(\"hello\");".to_string()),
            ],
        }];
        let updated = apply_hunks_to_content("fn  greet() {\nprintln!(\"hi\");\n}\n", &hunks)
            .expect("fuzzy hunk should apply");
        assert!(updated.contains("println!(\"hello\")"));
    }
}
