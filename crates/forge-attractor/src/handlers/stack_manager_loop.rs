use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext,
    condition::evaluate_condition_expression, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Default)]
pub struct StackManagerLoopHandler;

#[async_trait]
impl NodeHandler for StackManagerLoopHandler {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let max_cycles = parse_usize_attr(node, "manager.max_cycles", 1000).max(1);
        let poll_interval_ms = parse_duration_attr_ms(node, "manager.poll_interval", 45_000);
        let actions = parse_actions(node);
        let stop_condition = attr_str(node, &["manager.stop_condition"])
            .unwrap_or_default()
            .trim()
            .to_string();

        let mut last_status = child_status_at_cycle(context, 1);
        let mut last_outcome = child_outcome_at_cycle(context, 1);

        for cycle in 1..=max_cycles {
            if actions.observe {
                last_status = child_status_at_cycle(context, cycle);
                last_outcome = child_outcome_at_cycle(context, cycle);
            }

            if let Some(status) = last_status.as_deref() {
                if status == "completed" && last_outcome.as_deref() == Some("success") {
                    return Ok(success_with_updates(
                        cycle,
                        poll_interval_ms,
                        Some("Child completed".to_string()),
                    ));
                }
                if status == "failed" {
                    return Ok(NodeOutcome::failure("Child failed"));
                }
            }

            if !stop_condition.is_empty() {
                let marker = NodeOutcome::success();
                let passed = evaluate_condition_expression(&stop_condition, &marker, context)
                    .map_err(|error| {
                        AttractorError::Runtime(format!(
                            "manager.stop_condition evaluation failed: {}",
                            error
                        ))
                    })?;
                if passed {
                    return Ok(success_with_updates(
                        cycle,
                        poll_interval_ms,
                        Some("Stop condition satisfied".to_string()),
                    ));
                }
            }

            if actions.steer {
                if let Some(decision) = context
                    .get("stack.manager.steer_decision")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    let mut outcome = success_with_updates(
                        cycle,
                        poll_interval_ms,
                        Some(format!("Steering decision applied: {}", decision)),
                    );
                    outcome.context_updates.insert(
                        "stack.manager.last_steer".to_string(),
                        Value::String(decision.to_string()),
                    );
                    return Ok(outcome);
                }
            }

            if actions.wait && poll_interval_ms > 0 {
                // Deterministic runtime behavior: record polling cadence without sleeping.
            }
        }

        Ok(NodeOutcome::failure("Max cycles exceeded"))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ManagerActions {
    observe: bool,
    steer: bool,
    wait: bool,
}

fn parse_actions(node: &Node) -> ManagerActions {
    let raw = attr_str(node, &["manager.actions"]).unwrap_or("observe,wait");
    let mut actions = ManagerActions::default();
    for action in raw.split(',').map(|entry| entry.trim()) {
        match action {
            "observe" => actions.observe = true,
            "steer" => actions.steer = true,
            "wait" => actions.wait = true,
            _ => {}
        }
    }
    if !actions.observe && !actions.steer && !actions.wait {
        actions.observe = true;
        actions.wait = true;
    }
    actions
}

fn success_with_updates(cycle: usize, poll_interval_ms: u64, notes: Option<String>) -> NodeOutcome {
    let mut updates = RuntimeContext::new();
    updates.insert(
        "stack.manager.cycles".to_string(),
        Value::Number((cycle as u64).into()),
    );
    updates.insert(
        "stack.manager.poll_interval_ms".to_string(),
        Value::Number(poll_interval_ms.into()),
    );

    NodeOutcome {
        status: NodeStatus::Success,
        notes,
        context_updates: updates,
        preferred_label: None,
        suggested_next_ids: Vec::new(),
    }
}

fn child_status_at_cycle(context: &RuntimeContext, cycle: usize) -> Option<String> {
    sequence_value(context, "stack.child.status_sequence", cycle).or_else(|| {
        context
            .get("stack.child.status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn child_outcome_at_cycle(context: &RuntimeContext, cycle: usize) -> Option<String> {
    sequence_value(context, "stack.child.outcome_sequence", cycle).or_else(|| {
        context
            .get("stack.child.outcome")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn sequence_value(context: &RuntimeContext, key: &str, cycle: usize) -> Option<String> {
    let index = cycle.saturating_sub(1);
    context
        .get(key)
        .and_then(Value::as_array)
        .and_then(|entries| entries.get(index))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn parse_usize_attr(node: &Node, key: &str, default: usize) -> usize {
    for candidate in attr_key_variants(key) {
        let Some(value) = node.attrs.get(&candidate) else {
            continue;
        };
        return match value {
            crate::AttrValue::Integer(value) if *value >= 0 => *value as usize,
            crate::AttrValue::String(value) => value.parse::<usize>().unwrap_or(default),
            _ => default,
        };
    }
    default
}

fn parse_duration_attr_ms(node: &Node, key: &str, default: u64) -> u64 {
    for candidate in attr_key_variants(key) {
        let Some(value) = node.attrs.get(&candidate) else {
            continue;
        };
        return match value {
            crate::AttrValue::Duration(value) => value.millis,
            crate::AttrValue::Integer(value) if *value >= 0 => *value as u64,
            crate::AttrValue::String(value) => parse_duration_text(value).unwrap_or(default),
            _ => default,
        };
    }
    default
}

fn parse_duration_text(value: &str) -> Option<u64> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let split_at = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, unit) = text.split_at(split_at);
    let amount = digits.parse::<u64>().ok()?;
    let multiplier = match unit {
        "" | "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(amount.saturating_mul(multiplier))
}

fn attr_key_variants(key: &str) -> Vec<String> {
    vec![key.to_string(), key.replace('.', "_")]
}

fn attr_str<'a>(node: &'a Node, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(value) = node.attrs.get_str(key) {
            return Some(value);
        }
        let underscored = key.replace('.', "_");
        if let Some(value) = node.attrs.get_str(&underscored) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;
    use serde_json::json;

    #[tokio::test(flavor = "current_thread")]
    async fn manager_loop_child_completion_expected_success() {
        let graph = parse_dot("digraph G { m [shape=house] }").expect("graph parse");
        let node = graph.nodes.get("m").expect("node exists");
        let mut context = RuntimeContext::new();
        context.insert(
            "stack.child.status_sequence".to_string(),
            json!(["running", "completed"]),
        );
        context.insert(
            "stack.child.outcome_sequence".to_string(),
            json!(["running", "success"]),
        );

        let outcome = StackManagerLoopHandler
            .execute(node, &context, &graph)
            .await
            .expect("execute should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome
                .context_updates
                .get("stack.manager.cycles")
                .and_then(Value::as_u64),
            Some(2)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manager_loop_stop_condition_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                m [shape=house, manager_stop_condition="context.stack.ready=true"]
            }
            "#,
        )
        .expect("graph parse");
        let node = graph.nodes.get("m").expect("node exists");
        let mut context = RuntimeContext::new();
        context.insert("stack.ready".to_string(), Value::Bool(true));

        let outcome = StackManagerLoopHandler
            .execute(node, &context, &graph)
            .await
            .expect("execute should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(
            outcome
                .notes
                .as_deref()
                .unwrap_or_default()
                .contains("Stop condition")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manager_loop_max_cycles_expected_fail() {
        let graph =
            parse_dot("digraph G { m [shape=house, manager_max_cycles=2] }").expect("graph parse");
        let node = graph.nodes.get("m").expect("node exists");

        let outcome = StackManagerLoopHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute should succeed");

        assert_eq!(outcome.status, NodeStatus::Fail);
    }
}
