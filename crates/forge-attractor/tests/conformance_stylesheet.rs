use forge_attractor::{AttrValue, AttractorError, Graph, Transform, prepare_pipeline};

struct AddTagTransform;

impl Transform for AddTagTransform {
    fn apply(&self, graph: &mut Graph) -> Result<(), AttractorError> {
        let node = graph.nodes.get_mut("review").expect("node should exist");
        node.attrs
            .set_inherited("context_tag", AttrValue::String("transformed".to_string()));
        Ok(())
    }
}

#[test]
fn conformance_stylesheet_and_transforms_expected_specificity_variable_expansion_and_custom_order()
{
    let dot = r#"
        digraph G {
            graph [
                goal="Ship feature",
                model_stylesheet="
                    * { llm_model: \"model-shape\"; }
                    .critical { llm_model: \"model-class\"; }
                    #review { reasoning_effort: \"high\"; }
                "
            ]

            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan for $goal", class="critical"]
            review [shape=box, prompt="Review plan"]
            exit [shape=Msquare]
            start -> plan -> review -> exit
        }
    "#;

    let (graph, diagnostics) = prepare_pipeline(dot, &[&AddTagTransform], &[])
        .expect("pipeline preparation should succeed");

    assert!(diagnostics.iter().all(|diag| !diag.is_error()));

    let plan = graph.nodes.get("plan").expect("plan should exist");
    assert_eq!(
        plan.attrs.get_str("prompt"),
        Some("Plan for Ship feature"),
        "built-in variable expansion should run"
    );
    assert_eq!(
        plan.attrs.get_str("llm_model"),
        Some("model-class"),
        "class selector should override shape selector"
    );

    let review = graph.nodes.get("review").expect("review should exist");
    assert_eq!(review.attrs.get_str("reasoning_effort"), Some("high"));
    assert_eq!(review.attrs.get_str("context_tag"), Some("transformed"));
}
