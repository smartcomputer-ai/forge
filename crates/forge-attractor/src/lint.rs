use crate::{Diagnostic, Graph, Severity, ValidationError, parse_stylesheet};
use std::collections::{BTreeSet, VecDeque};

pub trait LintRule {
    fn name(&self) -> &str;
    fn apply(&self, graph: &Graph) -> Vec<Diagnostic>;
}

pub fn validate(graph: &Graph, extra_rules: &[&dyn LintRule]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    diagnostics.extend(rule_start_node(graph));
    diagnostics.extend(rule_terminal_node(graph));
    diagnostics.extend(rule_edge_target_exists(graph));
    diagnostics.extend(rule_start_no_incoming(graph));
    diagnostics.extend(rule_exit_no_outgoing(graph));
    diagnostics.extend(rule_reachability(graph));
    diagnostics.extend(rule_condition_syntax(graph));
    diagnostics.extend(rule_stylesheet_syntax(graph));
    diagnostics.extend(rule_type_known(graph));
    diagnostics.extend(rule_fidelity_valid(graph));
    diagnostics.extend(rule_retry_target_exists(graph));
    diagnostics.extend(rule_goal_gate_has_retry(graph));
    diagnostics.extend(rule_prompt_on_llm_nodes(graph));

    for rule in extra_rules {
        diagnostics.extend(rule.apply(graph));
    }

    diagnostics
}

pub fn validate_or_raise(
    graph: &Graph,
    extra_rules: &[&dyn LintRule],
) -> Result<Vec<Diagnostic>, ValidationError> {
    let diagnostics = validate(graph, extra_rules);
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(ValidationError::new(diagnostics));
    }
    Ok(diagnostics)
}

fn rule_start_node(graph: &Graph) -> Vec<Diagnostic> {
    let starts = graph.start_candidates();
    if starts.len() == 1 {
        Vec::new()
    } else {
        vec![Diagnostic::new(
            "start_node",
            Severity::Error,
            format!(
                "pipeline must have exactly one start node; found {}",
                starts.len()
            ),
        )]
    }
}

fn rule_terminal_node(graph: &Graph) -> Vec<Diagnostic> {
    let exits = graph.terminal_candidates();
    if exits.is_empty() {
        vec![Diagnostic::new(
            "terminal_node",
            Severity::Error,
            "pipeline must have at least one terminal node",
        )]
    } else {
        Vec::new()
    }
}

fn rule_edge_target_exists(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for edge in &graph.edges {
        if !graph.nodes.contains_key(&edge.to) {
            diagnostics.push(
                Diagnostic::new(
                    "edge_target_exists",
                    Severity::Error,
                    format!("edge target '{}' does not exist", edge.to),
                )
                .with_edge(edge.from.clone(), edge.to.clone()),
            );
        }
    }
    diagnostics
}

fn rule_start_no_incoming(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for start in graph.start_candidates() {
        if graph.incoming_edges(&start.id).next().is_some() {
            diagnostics.push(
                Diagnostic::new(
                    "start_no_incoming",
                    Severity::Error,
                    "start node must have no incoming edges",
                )
                .with_node_id(start.id.clone()),
            );
        }
    }
    diagnostics
}

fn rule_exit_no_outgoing(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for terminal in graph.terminal_candidates() {
        if graph.outgoing_edges(&terminal.id).next().is_some() {
            diagnostics.push(
                Diagnostic::new(
                    "exit_no_outgoing",
                    Severity::Error,
                    "terminal node must have no outgoing edges",
                )
                .with_node_id(terminal.id.clone()),
            );
        }
    }
    diagnostics
}

fn rule_reachability(graph: &Graph) -> Vec<Diagnostic> {
    let Some(start) = graph.start_candidates().into_iter().next() else {
        return Vec::new();
    };

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start.id.clone());
    queue.push_back(start.id.clone());

    while let Some(node_id) = queue.pop_front() {
        for edge in graph.outgoing_edges(&node_id) {
            if visited.insert(edge.to.clone()) {
                queue.push_back(edge.to.clone());
            }
        }
    }

    let mut diagnostics = Vec::new();
    for node in graph.nodes.values() {
        if !visited.contains(&node.id) {
            diagnostics.push(
                Diagnostic::new(
                    "reachability",
                    Severity::Error,
                    "node is unreachable from start",
                )
                .with_node_id(node.id.clone()),
            );
        }
    }
    diagnostics
}

fn rule_condition_syntax(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for edge in &graph.edges {
        let condition = edge.attrs.get_str("condition").unwrap_or_default();
        if condition.is_empty() {
            continue;
        }

        if let Err(message) = validate_condition_expression(condition) {
            diagnostics.push(
                Diagnostic::new("condition_syntax", Severity::Error, message)
                    .with_edge(edge.from.clone(), edge.to.clone()),
            );
        }
    }

    diagnostics
}

fn rule_stylesheet_syntax(graph: &Graph) -> Vec<Diagnostic> {
    let stylesheet = graph.attrs.get_str("model_stylesheet").unwrap_or_default();
    if stylesheet.trim().is_empty() {
        return Vec::new();
    }

    match parse_stylesheet(stylesheet) {
        Ok(_) => Vec::new(),
        Err(error) => vec![Diagnostic::new(
            "stylesheet_syntax",
            Severity::Error,
            error.to_string(),
        )],
    }
}

fn rule_type_known(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let known = known_types();

    for node in graph.nodes.values() {
        if let Some(node_type) = node.attrs.get_str("type") {
            if !known.contains(node_type) {
                diagnostics.push(
                    Diagnostic::new(
                        "type_known",
                        Severity::Warning,
                        format!("unknown node type '{node_type}'"),
                    )
                    .with_node_id(node.id.clone()),
                );
            }
        }
    }

    diagnostics
}

fn rule_fidelity_valid(graph: &Graph) -> Vec<Diagnostic> {
    let allowed: BTreeSet<&str> = [
        "full",
        "truncate",
        "compact",
        "summary:low",
        "summary:medium",
        "summary:high",
    ]
    .into_iter()
    .collect();

    let mut diagnostics = Vec::new();

    if let Some(value) = graph.attrs.get_str("default_fidelity") {
        if !value.is_empty() && !allowed.contains(value) {
            diagnostics.push(Diagnostic::new(
                "fidelity_valid",
                Severity::Warning,
                format!("graph default_fidelity '{value}' is invalid"),
            ));
        }
    }

    for node in graph.nodes.values() {
        if let Some(value) = node.attrs.get_str("fidelity") {
            if !value.is_empty() && !allowed.contains(value) {
                diagnostics.push(
                    Diagnostic::new(
                        "fidelity_valid",
                        Severity::Warning,
                        format!("node fidelity '{value}' is invalid"),
                    )
                    .with_node_id(node.id.clone()),
                );
            }
        }
    }

    for edge in &graph.edges {
        if let Some(value) = edge.attrs.get_str("fidelity") {
            if !value.is_empty() && !allowed.contains(value) {
                diagnostics.push(
                    Diagnostic::new(
                        "fidelity_valid",
                        Severity::Warning,
                        format!("edge fidelity '{value}' is invalid"),
                    )
                    .with_edge(edge.from.clone(), edge.to.clone()),
                );
            }
        }
    }

    diagnostics
}

fn rule_retry_target_exists(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for key in ["retry_target", "fallback_retry_target"] {
        if let Some(target) = graph.attrs.get_str(key) {
            if !target.is_empty() && !graph.nodes.contains_key(target) {
                diagnostics.push(Diagnostic::new(
                    "retry_target_exists",
                    Severity::Warning,
                    format!("graph {key} references missing node '{target}'"),
                ));
            }
        }
    }

    for node in graph.nodes.values() {
        for key in ["retry_target", "fallback_retry_target"] {
            if let Some(target) = node.attrs.get_str(key) {
                if !target.is_empty() && !graph.nodes.contains_key(target) {
                    diagnostics.push(
                        Diagnostic::new(
                            "retry_target_exists",
                            Severity::Warning,
                            format!("node {key} references missing node '{target}'"),
                        )
                        .with_node_id(node.id.clone()),
                    );
                }
            }
        }
    }

    diagnostics
}

fn rule_goal_gate_has_retry(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for node in graph.nodes.values() {
        if node.attrs.get_bool("goal_gate") == Some(true) {
            let has_retry_target = !node
                .attrs
                .get_str("retry_target")
                .unwrap_or_default()
                .is_empty();
            let has_fallback = !node
                .attrs
                .get_str("fallback_retry_target")
                .unwrap_or_default()
                .is_empty();

            if !has_retry_target && !has_fallback {
                diagnostics.push(
                    Diagnostic::new(
                        "goal_gate_has_retry",
                        Severity::Warning,
                        "goal_gate node should define retry_target or fallback_retry_target",
                    )
                    .with_node_id(node.id.clone()),
                );
            }
        }
    }

    diagnostics
}

fn rule_prompt_on_llm_nodes(graph: &Graph) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for node in graph.nodes.values() {
        if resolved_handler_type(node) == "codergen" {
            let has_prompt = !node.attrs.get_str("prompt").unwrap_or_default().is_empty();
            let has_label = !node.attrs.get_str("label").unwrap_or_default().is_empty();
            if !has_prompt && !has_label {
                diagnostics.push(
                    Diagnostic::new(
                        "prompt_on_llm_nodes",
                        Severity::Warning,
                        "codergen node should define prompt or label",
                    )
                    .with_node_id(node.id.clone()),
                );
            }
        }
    }

    diagnostics
}

fn validate_condition_expression(condition: &str) -> Result<(), String> {
    for clause in condition.split("&&") {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }

        let (key, value_opt) = if let Some((left, right)) = clause.split_once("!=") {
            (left.trim(), Some(right.trim()))
        } else if let Some((left, right)) = clause.split_once('=') {
            (left.trim(), Some(right.trim()))
        } else {
            (clause, None)
        };

        if key.is_empty() {
            return Err(format!("condition clause '{clause}' has empty key"));
        }
        if !is_condition_key(key) {
            return Err(format!("condition key '{key}' is invalid"));
        }

        if let Some(value) = value_opt {
            if value.is_empty() {
                return Err(format!("condition clause '{clause}' has empty value"));
            }
        }
    }

    Ok(())
}

fn is_condition_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}

fn known_types() -> BTreeSet<&'static str> {
    [
        "start",
        "exit",
        "codergen",
        "wait.human",
        "conditional",
        "parallel",
        "parallel.fan_in",
        "tool",
        "stack.manager_loop",
    ]
    .into_iter()
    .collect()
}

fn resolved_handler_type(node: &crate::Node) -> &'static str {
    if let Some(node_type) = node.attrs.get_str("type") {
        if !node_type.is_empty() {
            return if matches!(
                node_type,
                "start"
                    | "exit"
                    | "codergen"
                    | "wait.human"
                    | "conditional"
                    | "parallel"
                    | "parallel.fan_in"
                    | "tool"
                    | "stack.manager_loop"
            ) {
                // known types are represented verbatim; unknown types are linted elsewhere.
                match node_type {
                    "start" => "start",
                    "exit" => "exit",
                    "codergen" => "codergen",
                    "wait.human" => "wait.human",
                    "conditional" => "conditional",
                    "parallel" => "parallel",
                    "parallel.fan_in" => "parallel.fan_in",
                    "tool" => "tool",
                    "stack.manager_loop" => "stack.manager_loop",
                    _ => "codergen",
                }
            } else {
                "codergen"
            };
        }
    }

    match node.attrs.get_str("shape").unwrap_or("box") {
        "Mdiamond" => "start",
        "Msquare" => "exit",
        "box" => "codergen",
        "hexagon" => "wait.human",
        "diamond" => "conditional",
        "component" => "parallel",
        "tripleoctagon" => "parallel.fan_in",
        "parallelogram" => "tool",
        "house" => "stack.manager_loop",
        _ => "codergen",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[test]
    fn validate_missing_start_node_expected_error() {
        let graph = parse_dot("digraph G { exit [shape=Msquare] }").expect("graph should parse");
        let diagnostics = validate(&graph, &[]);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "start_node" && d.is_error())
        );
    }

    #[test]
    fn validate_invalid_condition_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit [condition="outcome="]
            }
            "#,
        )
        .expect("graph should parse");

        let diagnostics = validate(&graph, &[]);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "condition_syntax" && d.is_error())
        );
    }

    #[test]
    fn validate_stylesheet_syntax_invalid_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="* { llm_model base; }"]
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let diagnostics = validate(&graph, &[]);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "stylesheet_syntax" && d.is_error())
        );
    }

    #[test]
    fn validate_or_raise_with_errors_expected_err() {
        let graph = parse_dot("digraph G { orphan }").expect("graph should parse");
        let error = validate_or_raise(&graph, &[]).expect_err("validation should fail");
        assert!(error.errors_count > 0);
    }

    #[test]
    fn validate_goal_gate_without_retry_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [goal_gate=true]
                exit [shape=Msquare]
                start -> gate -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let diagnostics = validate(&graph, &[]);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "goal_gate_has_retry" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_prompt_on_llm_node_missing_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                task
                exit [shape=Msquare]
                start -> task -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let diagnostics = validate(&graph, &[]);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "prompt_on_llm_nodes" && d.severity == Severity::Warning)
        );
    }
}
