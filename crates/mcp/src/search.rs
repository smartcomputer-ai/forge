//! Deterministic lexical search over live MCP tool inventories.
//!
//! The matcher is intentionally independent of inventory transport, storage,
//! pagination, and model-facing result rendering so every runtime can apply
//! the same search policy.

/// A normalized, reusable MCP tool search query.
#[derive(Clone, Debug)]
pub struct McpToolSearchQuery {
    normalized: String,
    terms: Vec<String>,
}

impl McpToolSearchQuery {
    pub fn new(value: &str) -> Self {
        let normalized = normalize_search_text(value);
        let terms = normalized.split_whitespace().map(str::to_owned).collect();
        Self { normalized, terms }
    }

    /// Score one tool against this query. `None` means that no query term
    /// matched strongly enough for the tool to be returned.
    pub fn score(&self, tool_name: &str, description: Option<&str>) -> Option<McpToolSearchScore> {
        if self.terms.is_empty() {
            return None;
        }
        let normalized_name = normalize_search_text(tool_name);
        let normalized_description = normalize_search_text(description.unwrap_or_default());
        let name_terms = search_name_terms(&normalized_name);
        let description_terms = normalized_description
            .split_whitespace()
            .collect::<Vec<_>>();
        let mut score = McpToolSearchScore {
            exact_name: normalized_name == self.normalized,
            phrase_in_name: normalized_name.contains(&self.normalized),
            ..McpToolSearchScore::default()
        };
        for query_term in &self.terms {
            let name_score = name_terms
                .iter()
                .filter_map(|candidate| term_score(query_term, candidate))
                .max();
            let description_score = description_terms
                .iter()
                .filter_map(|candidate| term_score(query_term, candidate))
                .max()
                .map(|score| score.saturating_sub(10));
            let Some(term_score) = name_score.into_iter().chain(description_score).max() else {
                continue;
            };
            score.matched_terms = score.matched_terms.saturating_add(1);
            score.name_terms = score
                .name_terms
                .saturating_add(u16::from(name_score.is_some()));
            score.score = score.score.saturating_add(term_score);
        }
        (score.matched_terms > 0).then_some(score)
    }
}

/// Opaque deterministic ranking key. Higher scores sort before lower scores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct McpToolSearchScore {
    exact_name: bool,
    phrase_in_name: bool,
    matched_terms: u16,
    name_terms: u16,
    score: u32,
}

fn search_name_terms(normalized: &str) -> Vec<String> {
    let base = normalized
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut terms = base.clone();
    // Keep compact aliases for adjacent camel-case words so `GitHub` is
    // searchable as both `git hub` and `github` without a language stemmer.
    for width in 2..=3 {
        for window in base.windows(width) {
            terms.push(window.concat());
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn term_score(query: &str, candidate: &str) -> Option<u32> {
    if query == candidate {
        return Some(100);
    }
    let query_len = query.chars().count();
    let candidate_len = candidate.chars().count();
    let shorter = query_len.min(candidate_len);
    let length_delta = query_len.abs_diff(candidate_len);
    if shorter >= 3
        && length_delta <= 3
        && (query.starts_with(candidate) || candidate.starts_with(query))
    {
        return Some(96_u32.saturating_sub(length_delta as u32));
    }
    if shorter < 4 {
        return None;
    }
    let similarity = strsim::jaro_winkler(query, candidate);
    (similarity >= 0.88).then_some((similarity * 90.0).round() as u32)
}

fn normalize_search_text(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut previous_was_lowercase_or_digit = false;
    for character in value.chars() {
        if character.is_uppercase() && previous_was_lowercase_or_digit {
            expanded.push(' ');
        }
        if character.is_alphanumeric() {
            expanded.extend(character.to_lowercase());
            previous_was_lowercase_or_digit = character.is_lowercase() || character.is_numeric();
        } else {
            expanded.push(' ');
            previous_was_lowercase_or_digit = false;
        }
    }
    expanded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators_and_camel_case() {
        assert_eq!(
            normalize_search_text("SearchGitHub-Issues.v2"),
            "search git hub issues v2"
        );
        let score = McpToolSearchQuery::new("github issues")
            .score("SearchGitHubIssues", None)
            .expect("camel-case acronym alias");
        assert_eq!(score.matched_terms, 2);
    }

    #[test]
    fn matches_plurals_partial_queries_and_typos() {
        let plural = McpToolSearchQuery::new("issues")
            .score("issue", None)
            .expect("plural-prefix match");
        assert_eq!(plural.matched_terms, 1);
        assert_eq!(plural.name_terms, 1);

        let query = McpToolSearchQuery::new("search issues");
        let full = query
            .score("search_issues", None)
            .expect("full token match");
        let partial = query.score("search", None).expect("partial token match");
        assert!(full > partial);
        assert_eq!(partial.matched_terms, 1);

        let typo = McpToolSearchQuery::new("serach issues")
            .score("search_issue", None)
            .expect("typo-tolerant match");
        assert_eq!(typo.matched_terms, 2);
        assert!(
            McpToolSearchQuery::new("calendar")
                .score("search_issue", None)
                .is_none()
        );
    }

    #[test]
    fn prefers_names_then_uses_descriptions() {
        let query = McpToolSearchQuery::new("customer lookup");
        let name_match = query
            .score("customer_lookup", Some("Find a record"))
            .expect("name match");
        let description_match = query
            .score("find_record", Some("Look up a customer account"))
            .expect("description match");
        assert!(name_match > description_match);
    }
}
