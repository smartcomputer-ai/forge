use forge_attractor::{parse_dot, AttrValue, DurationValue};

#[test]
fn parse_dot_chained_edges_and_defaults_expected_expansion_and_inheritance() {
    let graph = parse_dot(
        r#"
        digraph G {
            node [timeout=15m]
            edge [weight=2]
            start [shape=Mdiamond]
            a
            b
            exit [shape=Msquare]
            start -> a -> b -> exit [label="next"]
        }
        "#,
    )
    .expect("graph should parse");

    assert_eq!(graph.edges.len(), 3);

    let a = graph.nodes.get("a").expect("node a should exist");
    assert!(matches!(
        a.attrs.get("timeout"),
        Some(AttrValue::Duration(DurationValue { millis: 900_000, .. }))
    ));

    for edge in &graph.edges {
        assert_eq!(edge.attrs.get("weight"), Some(&AttrValue::Integer(2)));
        assert_eq!(
            edge.attrs.get("label"),
            Some(&AttrValue::String("next".to_string()))
        );
    }
}

#[test]
fn parse_dot_subgraph_class_merge_expected_class_list() {
    let graph = parse_dot(
        r#"
        digraph G {
            subgraph cluster_loop {
                label="Loop A"
                Work [class="existing"]
            }
        }
        "#,
    )
    .expect("graph should parse");

    let node = graph.nodes.get("Work").expect("Work node should exist");
    assert_eq!(node.attrs.get_str("class"), Some("existing,loop-a"));
}
