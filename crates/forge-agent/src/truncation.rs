use crate::SessionConfig;

const CHAR_TRUNCATION_WARNING_PREFIX: &str = "[WARNING: Tool output was truncated.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TruncationMode {
    HeadTail,
    Tail,
}

pub fn default_truncation_mode_for_tool(tool_name: &str) -> TruncationMode {
    match tool_name {
        "grep" | "glob" | "edit_file" | "apply_patch" | "write_file" => TruncationMode::Tail,
        _ => TruncationMode::HeadTail,
    }
}

pub fn truncate_tool_output(output: &str, tool_name: &str, config: &SessionConfig) -> String {
    let max_chars = config
        .tool_output_limits
        .get(tool_name)
        .copied()
        .unwrap_or(20_000);
    let mode = default_truncation_mode_for_tool(tool_name);
    let mut truncated = truncate_chars(output, max_chars, mode);
    let char_warning_line = if output.chars().count() > max_chars {
        truncated
            .lines()
            .find(|line| line.starts_with(CHAR_TRUNCATION_WARNING_PREFIX))
            .map(ToOwned::to_owned)
    } else {
        None
    };

    if let Some(max_lines) = config.tool_line_limits.get(tool_name).copied() {
        truncated = truncate_lines(&truncated, max_lines);
        if let Some(warning_line) = char_warning_line.as_deref() {
            if !truncated.contains(CHAR_TRUNCATION_WARNING_PREFIX) {
                truncated = format!("{warning_line}\n\n{truncated}");
            }
        }
    }

    truncated
}

pub fn truncate_chars(output: &str, max_chars: usize, mode: TruncationMode) -> String {
    let char_count = output.chars().count();
    if char_count <= max_chars {
        return output.to_string();
    }

    let removed = char_count.saturating_sub(max_chars);
    match mode {
        TruncationMode::HeadTail => {
            let head = max_chars / 2;
            let tail = max_chars.saturating_sub(head);
            format!(
                "{}\n\n[WARNING: Tool output was truncated. {} characters were removed from the middle. The full output is available in the event stream. If you need to see specific parts, re-run the tool with more targeted parameters.]\n\n{}",
                take_head(output, head),
                removed,
                take_tail(output, tail)
            )
        }
        TruncationMode::Tail => {
            format!(
                "[WARNING: Tool output was truncated. First {} characters were removed. The full output is available in the event stream. If you need to see specific parts, re-run the tool with more targeted parameters.]\n\n{}",
                removed,
                take_tail(output, max_chars)
            )
        }
    }
}

pub fn truncate_lines(output: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= max_lines {
        return output.to_string();
    }

    let head_count = max_lines / 2;
    let tail_count = max_lines.saturating_sub(head_count);
    let omitted = lines.len().saturating_sub(head_count + tail_count);

    let head = lines[..head_count].join("\n");
    let tail = lines[lines.len() - tail_count..].join("\n");
    format!("{head}\n[... {omitted} lines omitted ...]\n{tail}")
}

fn take_head(input: &str, char_count: usize) -> String {
    input.chars().take(char_count).collect()
}

fn take_tail(input: &str, char_count: usize) -> String {
    let total = input.chars().count();
    input
        .chars()
        .skip(total.saturating_sub(char_count))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionConfig;

    #[test]
    fn truncate_chars_head_tail_marker_includes_removed_count_and_guidance() {
        let input = "abcdefghij";
        let output = truncate_chars(input, 6, TruncationMode::HeadTail);
        assert!(output.contains("[WARNING: Tool output was truncated."));
        assert!(output.contains("4 characters were removed from the middle"));
        assert!(output.contains("re-run the tool with more targeted parameters"));
    }

    #[test]
    fn truncate_lines_limits_visible_lines() {
        let input = "1\n2\n3\n4\n5\n6";
        let output = truncate_lines(input, 4);
        assert!(output.contains("lines omitted"));
    }

    #[test]
    fn truncate_tool_output_applies_character_limit_before_line_limit() {
        let mut config = SessionConfig::default();
        config.tool_output_limits.insert("shell".to_string(), 500);
        config.tool_line_limits.insert("shell".to_string(), 256);

        // Pathological case: a single huge line should still be reduced by char truncation.
        let input = "x".repeat(20_000);
        let output = truncate_tool_output(&input, "shell", &config);

        // Line truncation is a no-op for one line; char truncation must have already reduced output.
        assert!(output.chars().count() < input.chars().count());
    }

    #[test]
    fn truncate_tool_output_preserves_char_warning_when_line_truncation_runs() {
        let mut config = SessionConfig::default();
        config.tool_output_limits.insert("shell".to_string(), 200);
        config.tool_line_limits.insert("shell".to_string(), 4);
        let input = (0..500)
            .map(|idx| format!("line-{idx:03}"))
            .collect::<Vec<_>>()
            .join("\n");

        let output = truncate_tool_output(&input, "shell", &config);
        assert!(output.contains(CHAR_TRUNCATION_WARNING_PREFIX));
    }

    #[test]
    fn truncate_chars_tail_removes_from_front_and_keeps_suffix() {
        let input = "0123456789";
        let output = truncate_chars(input, 4, TruncationMode::Tail);
        assert!(output.contains("First 6 characters were removed"));
        assert!(output.ends_with("6789"));
    }
}
