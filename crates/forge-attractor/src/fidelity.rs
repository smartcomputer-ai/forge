use crate::{Edge, Graph};

pub const DEFAULT_FIDELITY: &str = "compact";

pub fn is_valid_fidelity_mode(mode: &str) -> bool {
    matches!(
        mode,
        "full" | "truncate" | "compact" | "summary:low" | "summary:medium" | "summary:high"
    )
}

pub fn find_incoming_edge<'a>(
    graph: &'a Graph,
    target_node_id: &str,
    previous_node_id: Option<&'a str>,
) -> Option<&'a Edge> {
    let from = previous_node_id?;
    graph
        .outgoing_edges(from)
        .find(|edge| edge.to == target_node_id)
}

pub fn resolve_fidelity_mode(
    graph: &Graph,
    target_node_id: &str,
    incoming_edge: Option<&Edge>,
) -> String {
    if let Some(edge) = incoming_edge {
        if let Some(fidelity) = edge.attrs.get_str("fidelity") {
            let trimmed = fidelity.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    if let Some(node) = graph.nodes.get(target_node_id) {
        if let Some(fidelity) = node.attrs.get_str("fidelity") {
            let trimmed = fidelity.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    if let Some(fidelity) = graph.attrs.get_str("default_fidelity") {
        let trimmed = fidelity.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    DEFAULT_FIDELITY.to_string()
}

pub fn resolve_thread_key(
    graph: &Graph,
    target_node_id: &str,
    incoming_edge: Option<&Edge>,
    previous_node_id: Option<&str>,
) -> Option<String> {
    let node = graph.nodes.get(target_node_id)?;

    if let Some(thread_id) = node.attrs.get_str("thread_id") {
        let trimmed = thread_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(edge) = incoming_edge {
        if let Some(thread_id) = edge.attrs.get_str("thread_id") {
            let trimmed = thread_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    for key in ["thread_id", "default_thread_id"] {
        if let Some(thread_id) = graph.attrs.get_str(key) {
            let trimmed = thread_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    if let Some(class_attr) = node.attrs.get_str("class") {
        if let Some(class_name) = class_attr
            .split(',')
            .map(|entry| entry.trim())
            .find(|entry| !entry.is_empty())
        {
            return Some(class_name.to_string());
        }
    }

    previous_node_id.map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[test]
    fn resolve_fidelity_mode_edge_precedence_expected_edge_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="compact"]
                start [shape=Mdiamond]
                plan [fidelity="summary:low"]
                start -> plan [fidelity="full"]
            }
            "#,
        )
        .expect("graph should parse");

        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(resolve_fidelity_mode(&graph, "plan", incoming), "full");
    }

    #[test]
    fn resolve_fidelity_mode_node_then_graph_then_default_expected_precedence() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="summary:medium"]
                start [shape=Mdiamond]
                plan [fidelity="truncate"]
                review
                start -> plan -> review
            }
            "#,
        )
        .expect("graph should parse");

        let incoming_plan = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_fidelity_mode(&graph, "plan", incoming_plan),
            "truncate"
        );

        let incoming_review = find_incoming_edge(&graph, "review", Some("plan"));
        assert_eq!(
            resolve_fidelity_mode(&graph, "review", incoming_review),
            "summary:medium"
        );
    }

    #[test]
    fn resolve_thread_key_precedence_expected_order() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [thread_id="graph-thread"]
                start [shape=Mdiamond]
                plan [thread_id="node-thread", class="loop-a"]
                review [class="review-cluster"]
                verify [class="verify-cluster"]
                start -> plan [thread_id="edge-thread"]
                plan -> review [thread_id="edge-review"]
                review -> verify
            }
            "#,
        )
        .expect("graph should parse");

        let incoming_plan = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming_plan, Some("start")).as_deref(),
            Some("node-thread")
        );

        let incoming_review = find_incoming_edge(&graph, "review", Some("plan"));
        assert_eq!(
            resolve_thread_key(&graph, "review", incoming_review, Some("plan")).as_deref(),
            Some("edge-review")
        );

        let graph_default = parse_dot(
            r#"
            digraph G {
                graph [thread_id="graph-thread"]
                start [shape=Mdiamond]
                review
                start -> review
            }
            "#,
        )
        .expect("graph should parse");
        let incoming = find_incoming_edge(&graph_default, "review", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph_default, "review", incoming, Some("start")).as_deref(),
            Some("graph-thread")
        );

        let class_derived = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                review [class="cluster-review"]
                start -> review
            }
            "#,
        )
        .expect("graph should parse");
        let incoming = find_incoming_edge(&class_derived, "review", Some("start"));
        assert_eq!(
            resolve_thread_key(&class_derived, "review", incoming, Some("start")).as_deref(),
            Some("cluster-review")
        );

        let fallback = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                review
                start -> review
            }
            "#,
        )
        .expect("graph should parse");
        let incoming = find_incoming_edge(&fallback, "review", Some("start"));
        assert_eq!(
            resolve_thread_key(&fallback, "review", incoming, Some("start")).as_deref(),
            Some("start")
        );
    }
}
