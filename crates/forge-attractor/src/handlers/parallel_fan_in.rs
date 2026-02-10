use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Candidate {
    id: String,
    status: NodeStatus,
    score: f64,
}

#[derive(Debug, Default)]
pub struct ParallelFanInHandler;

#[async_trait]
impl NodeHandler for ParallelFanInHandler {
    async fn execute(
        &self,
        _node: &Node,
        context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let Some(results) = context.get("parallel.results").and_then(Value::as_array) else {
            return Ok(NodeOutcome::failure(
                "No parallel results to evaluate".to_string(),
            ));
        };
        if results.is_empty() {
            return Ok(NodeOutcome::failure(
                "No parallel results to evaluate".to_string(),
            ));
        }

        let mut candidates: Vec<Candidate> =
            results.iter().filter_map(candidate_from_value).collect();
        if candidates.is_empty() {
            return Ok(NodeOutcome::failure(
                "No parseable parallel results to evaluate".to_string(),
            ));
        }

        candidates.sort_by(|left, right| {
            rank_status(left.status)
                .cmp(&rank_status(right.status))
                .then_with(|| {
                    right
                        .score
                        .partial_cmp(&left.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        let best = candidates
            .first()
            .expect("candidates should contain at least one entry")
            .clone();

        let all_failed = candidates
            .iter()
            .all(|candidate| candidate.status == NodeStatus::Fail);
        let status = if all_failed {
            NodeStatus::Fail
        } else {
            NodeStatus::Success
        };

        let mut updates = RuntimeContext::new();
        updates.insert(
            "parallel.fan_in.best_id".to_string(),
            Value::String(best.id.clone()),
        );
        updates.insert(
            "parallel.fan_in.best_outcome".to_string(),
            Value::String(best.status.as_str().to_string()),
        );
        updates.insert("parallel.fan_in.best_score".to_string(), json!(best.score));
        updates.insert(
            "parallel.fan_in.candidate_count".to_string(),
            Value::Number((candidates.len() as u64).into()),
        );

        Ok(NodeOutcome {
            status,
            notes: Some(format!(
                "Selected best candidate: {} ({})",
                best.id,
                best.status.as_str()
            )),
            context_updates: updates,
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        })
    }
}

fn candidate_from_value(value: &Value) -> Option<Candidate> {
    let object = value.as_object()?;
    let id = object
        .get("branch_id")
        .and_then(Value::as_str)
        .or_else(|| object.get("target_node").and_then(Value::as_str))?
        .to_string();
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .and_then(parse_status)
        .unwrap_or(NodeStatus::Fail);
    let score = object.get("score").and_then(Value::as_f64).unwrap_or(0.0);

    Some(Candidate { id, status, score })
}

fn parse_status(value: &str) -> Option<NodeStatus> {
    match value.trim() {
        "success" => Some(NodeStatus::Success),
        "partial_success" => Some(NodeStatus::PartialSuccess),
        "retry" => Some(NodeStatus::Retry),
        "fail" => Some(NodeStatus::Fail),
        _ => None,
    }
}

fn rank_status(status: NodeStatus) -> u8 {
    match status {
        NodeStatus::Success => 0,
        NodeStatus::PartialSuccess => 1,
        NodeStatus::Retry => 2,
        NodeStatus::Fail => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[tokio::test(flavor = "current_thread")]
    async fn fan_in_selects_best_candidate_expected_success() {
        let graph = parse_dot("digraph G { n1 [shape=tripleoctagon] }").expect("graph parse");
        let node = graph.nodes.get("n1").expect("node exists");
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.results".to_string(),
            json!([
                {"branch_id": "a", "status": "partial_success", "score": 0.4},
                {"branch_id": "b", "status": "success", "score": 0.1},
                {"branch_id": "c", "status": "success", "score": 0.9}
            ]),
        );

        let outcome = ParallelFanInHandler
            .execute(node, &context, &graph)
            .await
            .expect("execute should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome.context_updates.get("parallel.fan_in.best_id"),
            Some(&Value::String("c".to_string()))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_in_all_failed_expected_fail() {
        let graph = parse_dot("digraph G { n1 [shape=tripleoctagon] }").expect("graph parse");
        let node = graph.nodes.get("n1").expect("node exists");
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.results".to_string(),
            json!([
                {"branch_id": "a", "status": "fail", "score": 0.4},
                {"branch_id": "b", "status": "fail", "score": 0.9}
            ]),
        );

        let outcome = ParallelFanInHandler
            .execute(node, &context, &graph)
            .await
            .expect("execute should succeed");

        assert_eq!(outcome.status, NodeStatus::Fail);
    }
}
