use crate::{Edge, Graph, NodeOutcome, RuntimeContext, evaluate_condition_expression};

pub fn select_next_edge<'a>(
    graph: &'a Graph,
    from_node_id: &'a str,
    outcome: &NodeOutcome,
    context: &RuntimeContext,
) -> Option<&'a Edge> {
    let edges: Vec<&Edge> = graph.outgoing_edges(from_node_id).collect();
    if edges.is_empty() {
        return None;
    }

    // Step 1: condition match
    let mut condition_matched = Vec::new();
    for edge in &edges {
        let condition = edge.attrs.get_str("condition").unwrap_or_default().trim();
        if condition.is_empty() {
            continue;
        }
        if evaluate_condition_expression(condition, outcome, context).unwrap_or(false) {
            condition_matched.push(*edge);
        }
    }
    if !condition_matched.is_empty() {
        return best_by_weight_then_lexical(condition_matched.iter().copied());
    }

    // Eligible for steps 2 and 3: unconditional or condition=true
    let mut eligible = Vec::new();
    for edge in &edges {
        let condition = edge.attrs.get_str("condition").unwrap_or_default().trim();
        if condition.is_empty()
            || evaluate_condition_expression(condition, outcome, context).unwrap_or(false)
        {
            eligible.push(*edge);
        }
    }

    // Step 2: preferred label
    if let Some(preferred) = outcome.preferred_label.as_ref() {
        let preferred = normalize_label(preferred);
        if let Some(edge) = eligible.iter().find(|edge| {
            normalize_label(edge.attrs.get_str("label").unwrap_or_default()) == preferred
        }) {
            return Some(*edge);
        }
    }

    // Step 3: suggested next ids
    if !outcome.suggested_next_ids.is_empty() {
        for suggested in &outcome.suggested_next_ids {
            if let Some(edge) = eligible.iter().find(|edge| edge.to == *suggested) {
                return Some(*edge);
            }
        }
    }

    // Step 4/5: unconditional by weight then lexical
    let unconditional: Vec<&Edge> = edges
        .iter()
        .copied()
        .filter(|edge| {
            edge.attrs
                .get_str("condition")
                .unwrap_or_default()
                .trim()
                .is_empty()
        })
        .collect();
    if !unconditional.is_empty() {
        return best_by_weight_then_lexical(unconditional.iter().copied());
    }

    // Fallback: any edge by weight then lexical
    best_by_weight_then_lexical(edges.iter().copied())
}

fn best_by_weight_then_lexical<'a, I>(edges: I) -> Option<&'a Edge>
where
    I: IntoIterator<Item = &'a Edge>,
{
    edges.into_iter().max_by(|left, right| {
        edge_weight(left)
            .cmp(&edge_weight(right))
            .then_with(|| right.to.cmp(&left.to))
    })
}

fn edge_weight(edge: &Edge) -> i64 {
    edge.attrs
        .get("weight")
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn normalize_label(input: &str) -> String {
    let trimmed = input.trim().to_ascii_lowercase();

    if trimmed.starts_with('[') {
        if let Some((_, rest)) = trimmed.split_once(']') {
            return rest.trim().to_string();
        }
    }

    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphanumeric() && bytes[1] == b')' {
        return trimmed[2..].trim().to_string();
    }

    if bytes.len() >= 3 && bytes[0].is_ascii_alphanumeric() && bytes[1] == b' ' && bytes[2] == b'-'
    {
        return trimmed[3..].trim().to_string();
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeOutcome, NodeStatus, parse_dot};
    use std::collections::BTreeMap;

    fn base_outcome() -> NodeOutcome {
        NodeOutcome {
            status: NodeStatus::Success,
            notes: None,
            context_updates: BTreeMap::new(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        }
    }

    #[test]
    fn select_next_edge_condition_match_expected_priority() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                n1 -> a [condition="outcome=fail", weight=100]
                n1 -> b [condition="outcome=success", weight=0]
            }
            "#,
        )
        .expect("graph should parse");
        let outcome = base_outcome();
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "b");
    }

    #[test]
    fn select_next_edge_preferred_label_normalized_expected_match() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                yes
                no
                n1 -> yes [label="[Y] Yes"]
                n1 -> no [label="No"]
            }
            "#,
        )
        .expect("graph should parse");
        let mut outcome = base_outcome();
        outcome.preferred_label = Some("yes".to_string());
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "yes");
    }

    #[test]
    fn select_next_edge_weight_then_lexical_expected_deterministic() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                c
                n1 -> b [weight=1]
                n1 -> c [weight=1]
                n1 -> a [weight=2]
            }
            "#,
        )
        .expect("graph should parse");
        let outcome = base_outcome();
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "a");
    }

    #[test]
    fn select_next_edge_step3_suggested_ids_expected_match() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                n1 -> a
                n1 -> b
            }
            "#,
        )
        .expect("graph should parse");
        let mut outcome = base_outcome();
        outcome.suggested_next_ids = vec!["b".to_string(), "a".to_string()];
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "b");
    }

    #[test]
    fn select_next_edge_step2_preferred_label_beats_suggested_ids_expected_label_route() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                yes
                no
                n1 -> yes [label="Yes"]
                n1 -> no [label="No"]
            }
            "#,
        )
        .expect("graph should parse");
        let mut outcome = base_outcome();
        outcome.preferred_label = Some("No".to_string());
        outcome.suggested_next_ids = vec!["yes".to_string()];
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "no");
    }

    #[test]
    fn select_next_edge_step1_condition_beats_preferred_label_expected_condition_route() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                pass
                fail
                n1 -> pass [condition="outcome=success"]
                n1 -> fail [label="fail"]
            }
            "#,
        )
        .expect("graph should parse");
        let mut outcome = base_outcome();
        outcome.preferred_label = Some("fail".to_string());
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "pass");
    }

    #[test]
    fn select_next_edge_condition_matches_weight_then_lexical_expected_tiebreak() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                c
                n1 -> b [condition="outcome=success", weight=1]
                n1 -> c [condition="outcome=success", weight=1]
                n1 -> a [condition="outcome=success", weight=2]
            }
            "#,
        )
        .expect("graph should parse");
        let outcome = base_outcome();
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "a");
    }

    #[test]
    fn select_next_edge_unconditional_lexical_tie_expected_smallest_id() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                n1 -> b [weight=1]
                n1 -> a [weight=1]
            }
            "#,
        )
        .expect("graph should parse");
        let outcome = base_outcome();
        let context = RuntimeContext::new();

        let selected = select_next_edge(&graph, "n1", &outcome, &context).expect("edge expected");
        assert_eq!(selected.to, "a");
    }
}
