use forge_attractor::{Severity, parse_dot, validate, validate_or_raise};

#[test]
fn conformance_parsing_dot_subset_expected_attrs_defaults_and_comments_supported() {
    let graph = parse_dot(
        r#"
        // graph-level comment
        digraph G {
            graph [goal="ship", label="Pipeline", model_stylesheet="box { llm_model = \"mock\"; }"]
            node [shape=box, prompt="default prompt"]
            edge [weight=7]

            /* multi-line node attrs */
            start [shape=Mdiamond]
            A [
                label="Stage A",
                prompt="Line 1\nLine 2"
            ]
            B [class="fast"]
            exit [shape=Msquare]

            start -> A -> B [label="next"]
            B -> exit [condition="outcome=success"]
        }
        "#,
    )
    .expect("graph should parse");

    assert_eq!(graph.attrs.get_str("goal"), Some("ship"));
    assert_eq!(graph.attrs.get_str("label"), Some("Pipeline"));
    assert_eq!(
        graph.nodes.get("B").and_then(|n| n.attrs.get_str("prompt")),
        Some("default prompt")
    );
    assert_eq!(
        graph.edges.len(),
        3,
        "chained edge should expand to two edges plus explicit edge"
    );
    assert_eq!(graph.edges[0].attrs.get_str("label"), Some("next"));
    assert_eq!(
        graph.edges[0].attrs.get("weight"),
        Some(&forge_attractor::AttrValue::Integer(7))
    );
}

#[test]
fn conformance_validation_expected_error_contract_with_rule_severity_and_location() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            bad [shape=box]
            start -> bad
            bad -> missing_target
        }
        "#,
    )
    .expect("graph should parse");

    let diagnostics = validate(&graph, &[]);
    assert!(diagnostics.iter().any(|d| d.rule == "edge_target_exists"));
    assert!(diagnostics.iter().any(|d| d.severity == Severity::Error));
    assert!(diagnostics.iter().any(|d| d.edge.is_some()));

    let err = validate_or_raise(&graph, &[]).expect_err("validation should fail");
    assert!(err.errors_count >= 1);
}
