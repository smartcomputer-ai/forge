use forge_attractor::{apply_builtin_transforms, parse_dot, AttrValue};

#[test]
fn stylesheet_transform_explicit_attr_not_overridden_expected_explicit_kept() {
    let mut graph = parse_dot(
        r#"
        digraph G {
            graph [
                goal="Ship",
                model_stylesheet="
                    * { llm_model: base; llm_provider: openai; }
                    #plan { llm_model: override; reasoning_effort: high; }
                "
            ]
            plan [prompt="Plan for $goal", llm_model="explicit"]
        }
        "#,
    )
    .expect("graph should parse");

    apply_builtin_transforms(&mut graph).expect("transforms should apply");
    let plan = graph.nodes.get("plan").expect("node should exist");

    assert_eq!(plan.attrs.get_str("prompt"), Some("Plan for Ship"));
    assert_eq!(
        plan.attrs.get("llm_model"),
        Some(&AttrValue::String("explicit".to_string()))
    );
    assert_eq!(
        plan.attrs.get("llm_provider"),
        Some(&AttrValue::String("openai".to_string()))
    );
}
