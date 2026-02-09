use regex::{Regex, RegexBuilder};

use crate::ToolError;

pub(crate) fn apply_edit(
    content: &str,
    file_path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<(String, usize), ToolError> {
    let replacement_count = content.match_indices(old_string).count();
    if replacement_count > 0 {
        if replacement_count > 1 && !replace_all {
            return Err(ToolError::Execution(format!(
                "old_string is not unique in '{}': found {} matches; provide more context or set replace_all=true",
                file_path, replacement_count
            )));
        }

        let next_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        return Ok((next_content, replacement_count));
    }

    let fuzzy_regex = build_fuzzy_regex(old_string)?;
    let matches: Vec<(usize, usize)> = fuzzy_regex
        .find_iter(content)
        .take(128)
        .map(|m| (m.start(), m.end()))
        .collect();

    if matches.is_empty() {
        return Err(ToolError::Execution(format!(
            "old_string not found in '{}' (exact and fuzzy matching failed)",
            file_path
        )));
    }
    if matches.len() > 1 && !replace_all {
        return Err(ToolError::Execution(format!(
            "old_string is not unique in '{}': fuzzy match found {} locations; provide more context or set replace_all=true",
            file_path,
            matches.len()
        )));
    }

    let mut updated = content.to_string();
    let replaced = if replace_all { matches.len() } else { 1 };
    if replace_all {
        for (start, end) in matches.into_iter().rev() {
            updated.replace_range(start..end, new_string);
        }
    } else {
        let (start, end) = matches[0];
        updated.replace_range(start..end, new_string);
    }

    Ok((updated, replaced))
}

fn build_fuzzy_regex(old_string: &str) -> Result<Regex, ToolError> {
    if old_string.chars().count() > 20_000 {
        return Err(ToolError::Execution(
            "old_string too large for fuzzy matching; narrow the selection".to_string(),
        ));
    }

    let mut pattern = String::new();
    let mut in_ws = false;
    for ch in old_string.chars() {
        if ch.is_whitespace() {
            if !in_ws {
                pattern.push_str("\\s+");
                in_ws = true;
            }
            continue;
        }
        in_ws = false;
        pattern.push_str(&char_match_pattern(ch));
    }

    if pattern.is_empty() {
        return Err(ToolError::Execution(
            "old_string must include non-whitespace content".to_string(),
        ));
    }

    RegexBuilder::new(&pattern)
        .dot_matches_new_line(true)
        .multi_line(true)
        .build()
        .map_err(|error| {
            ToolError::Execution(format!(
                "failed to build fuzzy matcher for old_string: {}",
                error
            ))
        })
}

fn char_match_pattern(ch: char) -> String {
    match ch {
        '\'' | '\u{2018}' | '\u{2019}' | '\u{02BC}' => "['\u{2018}\u{2019}\u{02BC}]".to_string(),
        '"' | '\u{201C}' | '\u{201D}' => "[\"\u{201C}\u{201D}]".to_string(),
        '-' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => {
            "[-\u{2010}\u{2011}\u{2012}\u{2013}\u{2014}\u{2212}]".to_string()
        }
        _ => regex::escape(&ch.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_edit;

    #[test]
    fn apply_edit_exact_match_replaces_once() {
        let (updated, replaced) =
            apply_edit("a b", "f.txt", "a b", "x", false).expect("exact match should succeed");
        assert_eq!(updated, "x");
        assert_eq!(replaced, 1);
    }

    #[test]
    fn apply_edit_fuzzy_match_replaces_whitespace_variant() {
        let (updated, replaced) = apply_edit(
            "fn  main() {\n}\n",
            "f.txt",
            "fn main() {\n}",
            "fn run() {\n}",
            false,
        )
        .expect("fuzzy match should succeed");
        assert!(updated.contains("fn run() {"));
        assert_eq!(replaced, 1);
    }

    #[test]
    fn apply_edit_fuzzy_match_reports_ambiguity_without_replace_all() {
        let err = apply_edit("a  b\nx\na b\n", "f.txt", "a   b", "z", false)
            .expect_err("expected ambiguity");
        let message = err.to_string();
        assert!(message.contains("not unique"));
        assert!(message.contains("fuzzy match found"));
    }
}
