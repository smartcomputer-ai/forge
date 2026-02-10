use forge_attractor::{
    NodeOutcome, NodeStatus, RuntimeContext, evaluate_condition_expression, parse_dot,
    select_next_edge, validate_condition_expression,
};
use serde_json::Value;

fn outcome(status: NodeStatus, preferred_label: Option<&str>) -> NodeOutcome {
    NodeOutcome {
        status,
        notes: None,
        context_updates: RuntimeContext::new(),
        preferred_label: preferred_label.map(ToOwned::to_owned),
        suggested_next_ids: vec![],
    }
}

#[test]
fn condition_validate_invalid_expected_error() {
    let error = validate_condition_expression("bad=1").expect_err("should fail");
    assert!(error.contains("invalid"));
}

#[test]
fn condition_evaluate_compound_expected_true() {
    let mut context = RuntimeContext::new();
    context.insert("ok".to_string(), Value::Bool(true));
    context.insert("count".to_string(), Value::Number(2.into()));

    let matched = evaluate_condition_expression(
        "outcome=success && preferred_label=Yes && context.ok=true && context.count=2",
        &outcome(NodeStatus::Success, Some("Yes")),
        &context,
    )
    .expect("evaluation should succeed");
    assert!(matched);
}

#[test]
fn condition_evaluate_not_equals_expected_false_when_equal() {
    let context = RuntimeContext::new();
    let matched = evaluate_condition_expression(
        "outcome!=success",
        &outcome(NodeStatus::Success, None),
        &context,
    )
    .expect("evaluation should succeed");
    assert!(!matched);
}

#[test]
fn condition_evaluate_exists_expected_false_for_missing_context_key() {
    let context = RuntimeContext::new();
    let matched = evaluate_condition_expression(
        "context.missing",
        &outcome(NodeStatus::Success, None),
        &context,
    )
    .expect("evaluation should succeed");
    assert!(!matched);
}

#[test]
fn condition_routing_by_expression_expected_matching_edge_selected() {
    let graph = parse_dot(
        r#"
        digraph G {
            gate
            pass
            fail
            gate -> pass [condition="outcome=success && context.ok=true"]
            gate -> fail [condition="outcome!=success"]
        }
        "#,
    )
    .expect("graph should parse");

    let mut context = RuntimeContext::new();
    context.insert("ok".to_string(), Value::Bool(true));
    let selected = select_next_edge(
        &graph,
        "gate",
        &outcome(NodeStatus::Success, None),
        &context,
    )
    .expect("edge expected");
    assert_eq!(selected.to, "pass");
}
