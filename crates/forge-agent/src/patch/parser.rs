use crate::ToolError;

use super::types::{PatchHunk, PatchHunkLine, PatchOperation};

pub(crate) fn parse_apply_patch(patch: &str) -> Result<Vec<PatchOperation>, ToolError> {
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

#[cfg(test)]
mod tests {
    use super::parse_apply_patch;

    #[test]
    fn parse_apply_patch_accepts_simple_update() {
        let patch = "\
*** Begin Patch
*** Update File: a.txt
@@ replace
-one
+two
*** End Patch";

        let operations = parse_apply_patch(patch).expect("patch should parse");
        assert_eq!(operations.len(), 1);
    }

    #[test]
    fn parse_apply_patch_rejects_missing_end_marker() {
        let patch = "\
*** Begin Patch
*** Delete File: a.txt";
        let err = parse_apply_patch(patch).expect_err("parse should fail");
        assert!(err.to_string().contains("must end with '*** End Patch'"));
    }
}
