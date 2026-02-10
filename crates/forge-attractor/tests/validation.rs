use forge_attractor::{Severity, parse_dot, validate, validate_or_raise};

#[test]
fn validate_reachability_orphan_expected_error() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            exit [shape=Msquare]
            orphan
            start -> exit
        }
        "#,
    )
    .expect("graph should parse");

    let diagnostics = validate(&graph, &[]);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.rule == "reachability" && d.severity == Severity::Error)
    );
}

#[test]
fn validate_or_raise_valid_graph_expected_ok() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [prompt="Plan"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    )
    .expect("graph should parse");

    validate_or_raise(&graph, &[]).expect("graph should be valid");
}
