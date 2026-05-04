use crate::{
    AttractorError, Graph, Node, NodeExecutor, NodeOutcome, NodeStatus, RuntimeContext,
    handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone, Debug)]
struct BranchResult {
    branch_id: String,
    target_node: String,
    status: NodeStatus,
    score: f64,
    notes: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JoinPolicy {
    AllSuccess,
    AnySuccess,
    Quorum,
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorPolicy {
    Continue,
    FailFast,
    Ignore,
}

pub struct ParallelHandler {
    executor: Option<Arc<dyn NodeExecutor>>,
}

impl Default for ParallelHandler {
    fn default() -> Self {
        Self { executor: None }
    }
}

impl std::fmt::Debug for ParallelHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelHandler")
            .field("has_executor", &self.executor.is_some())
            .finish()
    }
}

impl ParallelHandler {
    pub fn with_executor(executor: Arc<dyn NodeExecutor>) -> Self {
        Self {
            executor: Some(executor),
        }
    }
}

#[async_trait]
impl NodeHandler for ParallelHandler {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let branches: Vec<(String, String)> = graph
            .outgoing_edges(&node.id)
            .map(|edge| {
                (
                    edge.attrs
                        .get_str("label")
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or(edge.to.as_str())
                        .to_string(),
                    edge.to.clone(),
                )
            })
            .collect();

        if branches.is_empty() {
            return Ok(NodeOutcome::failure(format!(
                "parallel node '{}' has no outgoing branches",
                node.id
            )));
        }

        let join_policy = parse_join_policy(node);
        let error_policy = parse_error_policy(node);
        let max_parallel = parse_usize_attr(node, "max_parallel", 4).max(1);
        let quorum_needed = quorum_target_count(node, branches.len());

        let mut results = if let Some(executor) = &self.executor {
            run_branch_batches_with_executor(
                branches,
                context,
                graph,
                executor.as_ref(),
                max_parallel,
                error_policy,
            )
            .await?
        } else {
            run_branch_batches_from_context(branches, context, max_parallel)?
        };
        results.sort_by(|left, right| left.branch_id.cmp(&right.branch_id));

        // error_policy=ignore: downgrade failures to success before join evaluation
        if error_policy == ErrorPolicy::Ignore {
            for result in &mut results {
                if result.status == NodeStatus::Fail {
                    result.status = NodeStatus::Success;
                }
            }
        }

        let success_count = results
            .iter()
            .filter(|result| result.status.is_success_like())
            .count();
        let fail_count = results
            .iter()
            .filter(|result| result.status == NodeStatus::Fail)
            .count();

        let (status, notes) = match join_policy {
            JoinPolicy::AllSuccess => {
                if fail_count == 0 {
                    (
                        NodeStatus::Success,
                        format!("all {} branches completed successfully", results.len()),
                    )
                } else {
                    (
                        NodeStatus::PartialSuccess,
                        format!(
                            "wait_all policy: {} of {} branches failed",
                            fail_count,
                            results.len()
                        ),
                    )
                }
            }
            JoinPolicy::AnySuccess => {
                if success_count > 0 {
                    (
                        NodeStatus::Success,
                        format!(
                            "any_success policy satisfied: {} successful branches",
                            success_count
                        ),
                    )
                } else {
                    (
                        NodeStatus::Fail,
                        "any_success policy failed: no successful branch".to_string(),
                    )
                }
            }
            JoinPolicy::Quorum => {
                if success_count >= quorum_needed {
                    (
                        NodeStatus::Success,
                        format!(
                            "quorum policy satisfied: {} successful branches (required {})",
                            success_count, quorum_needed
                        ),
                    )
                } else {
                    (
                        NodeStatus::Fail,
                        format!(
                            "quorum policy failed: {} successful branches (required {})",
                            success_count, quorum_needed
                        ),
                    )
                }
            }
            JoinPolicy::Ignore => (
                NodeStatus::Success,
                format!(
                    "ignore policy: {} branches completed ({} failures ignored)",
                    results.len(),
                    fail_count
                ),
            ),
        };

        let mut updates = RuntimeContext::new();
        updates.insert(
            "parallel.results".to_string(),
            Value::Array(results.iter().map(branch_result_to_value).collect()),
        );
        updates.insert(
            "parallel.branch_count".to_string(),
            Value::Number((results.len() as u64).into()),
        );
        updates.insert(
            "parallel.success_count".to_string(),
            Value::Number((success_count as u64).into()),
        );
        updates.insert(
            "parallel.fail_count".to_string(),
            Value::Number((fail_count as u64).into()),
        );
        updates.insert(
            "parallel.join_policy".to_string(),
            Value::String(join_policy.as_str().to_string()),
        );

        Ok(NodeOutcome {
            status,
            notes: Some(notes),
            context_updates: updates,
            ..Default::default()
        })
    }
}

impl JoinPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::AllSuccess => "all_success",
            Self::AnySuccess => "any_success",
            Self::Quorum => "quorum",
            Self::Ignore => "ignore",
        }
    }
}

/// Execute branches using a real NodeExecutor — each branch target node is executed
/// with an isolated context clone.
async fn run_branch_batches_with_executor(
    branches: Vec<(String, String)>,
    context: &RuntimeContext,
    graph: &Graph,
    executor: &dyn NodeExecutor,
    max_parallel: usize,
    error_policy: ErrorPolicy,
) -> Result<Vec<BranchResult>, AttractorError> {
    let mut out = Vec::with_capacity(branches.len());

    for batch in branches.chunks(max_parallel) {
        let mut futures = Vec::with_capacity(batch.len());
        for (branch_id, target_node) in batch {
            let local_context = branch_context(context, branch_id, target_node);
            let target = graph.nodes.get(target_node.as_str());
            let branch_id = branch_id.clone();
            let target_node = target_node.clone();

            if let Some(target_node_ref) = target {
                futures.push(async move {
                    match executor
                        .execute(target_node_ref, &local_context, graph)
                        .await
                    {
                        Ok(outcome) => BranchResult {
                            branch_id,
                            target_node,
                            status: outcome.status,
                            score: 0.0,
                            notes: outcome.notes,
                        },
                        Err(error) => BranchResult {
                            branch_id,
                            target_node,
                            status: NodeStatus::Fail,
                            score: 0.0,
                            notes: Some(error.to_string()),
                        },
                    }
                });
            } else {
                out.push(BranchResult {
                    branch_id,
                    target_node,
                    status: NodeStatus::Fail,
                    score: 0.0,
                    notes: Some("target node not found in graph".to_string()),
                });
            }
        }

        // Execute all futures in the batch concurrently
        let batch_results = futures::future::join_all(futures).await;
        out.extend(batch_results);

        // fail_fast: abort remaining batches on first failure
        if error_policy == ErrorPolicy::FailFast && out.iter().any(|r| r.status == NodeStatus::Fail)
        {
            break;
        }
    }

    Ok(out)
}

/// Context-driven branch resolution (backward compat for tests without executor)
fn run_branch_batches_from_context(
    branches: Vec<(String, String)>,
    context: &RuntimeContext,
    _max_parallel: usize,
) -> Result<Vec<BranchResult>, AttractorError> {
    let mut out = Vec::with_capacity(branches.len());
    for (branch_id, target_node) in &branches {
        let local_context = branch_context(context, branch_id, target_node);
        out.push(resolve_branch_result(
            branch_id,
            target_node,
            &local_context,
        ));
    }
    Ok(out)
}

fn branch_context(base: &RuntimeContext, branch_id: &str, target_node: &str) -> RuntimeContext {
    let mut cloned = base.clone();
    cloned.insert(
        "work.branch_id".to_string(),
        Value::String(branch_id.to_string()),
    );
    cloned.insert(
        "work.branch_target".to_string(),
        Value::String(target_node.to_string()),
    );
    cloned
}

fn resolve_branch_result(
    branch_id: &str,
    target_node: &str,
    context: &RuntimeContext,
) -> BranchResult {
    let status = context
        .get("parallel.branch_outcomes")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(branch_id))
        .and_then(Value::as_str)
        .and_then(parse_status)
        .or_else(|| {
            context
                .get("parallel.branch_outcomes")
                .and_then(Value::as_object)
                .and_then(|entries| entries.get(target_node))
                .and_then(Value::as_str)
                .and_then(parse_status)
        })
        .unwrap_or(NodeStatus::Success);

    let score = context
        .get("parallel.branch_scores")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(branch_id))
        .and_then(Value::as_f64)
        .or_else(|| {
            context
                .get("parallel.branch_scores")
                .and_then(Value::as_object)
                .and_then(|entries| entries.get(target_node))
                .and_then(Value::as_f64)
        })
        .unwrap_or(0.0);

    let notes = context
        .get("parallel.branch_notes")
        .and_then(Value::as_object)
        .and_then(|entries| entries.get(branch_id))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    BranchResult {
        branch_id: branch_id.to_string(),
        target_node: target_node.to_string(),
        status,
        score,
        notes,
    }
}

fn parse_join_policy(node: &Node) -> JoinPolicy {
    let value = attr_str(node, &["join_policy"]).unwrap_or("all_success");
    match value.trim() {
        "any_success" | "first_success" => JoinPolicy::AnySuccess,
        "quorum" | "k_of_n" => JoinPolicy::Quorum,
        "ignore" => JoinPolicy::Ignore,
        "all_success" | "wait_all" | _ => JoinPolicy::AllSuccess,
    }
}

fn parse_error_policy(node: &Node) -> ErrorPolicy {
    let value = attr_str(node, &["error_policy"]).unwrap_or("continue");
    match value.trim() {
        "fail_fast" => ErrorPolicy::FailFast,
        "ignore" => ErrorPolicy::Ignore,
        "continue" | _ => ErrorPolicy::Continue,
    }
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

fn parse_f64_attr(node: &Node, key: &str, default: f64) -> f64 {
    for candidate in attr_key_variants(key) {
        let Some(value) = node.attrs.get(&candidate) else {
            continue;
        };
        return match value {
            crate::AttrValue::Float(value) => *value,
            crate::AttrValue::Integer(value) => *value as f64,
            crate::AttrValue::String(value) => value.parse::<f64>().unwrap_or(default),
            _ => default,
        };
    }
    default
}

fn quorum_target_count(node: &Node, branch_count: usize) -> usize {
    for candidate in attr_key_variants("quorum_count") {
        if let Some(explicit) = node.attrs.get(&candidate).and_then(|value| match value {
            crate::AttrValue::Integer(value) if *value >= 1 => Some(*value as usize),
            crate::AttrValue::String(value) => value.parse::<usize>().ok(),
            _ => None,
        }) {
            return explicit.min(branch_count).max(1);
        }
    }

    let ratio = parse_f64_attr(node, "quorum_ratio", 0.5).clamp(0.0, 1.0);
    ((branch_count as f64) * ratio).ceil().max(1.0) as usize
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

fn parse_status(value: &str) -> Option<NodeStatus> {
    match value.trim() {
        "success" => Some(NodeStatus::Success),
        "partial_success" => Some(NodeStatus::PartialSuccess),
        "retry" => Some(NodeStatus::Retry),
        "fail" => Some(NodeStatus::Fail),
        "skipped" => Some(NodeStatus::Skipped),
        _ => None,
    }
}

fn branch_result_to_value(result: &BranchResult) -> Value {
    json!({
        "branch_id": result.branch_id,
        "target_node": result.target_node,
        "status": result.status.as_str(),
        "score": result.score,
        "notes": result.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_all_success_expected_success_and_results() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="all_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("p").expect("node should exist");

        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &RuntimeContext::new(),
            &graph,
        )
        .await
        .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome
                .context_updates
                .get("parallel.branch_count")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert!(outcome.context_updates.contains_key("parallel.results"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_any_success_with_failures_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="any_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("p").expect("node should exist");
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "success"}),
        );

        let outcome = NodeHandler::execute(&ParallelHandler::default(), node, &context, &graph)
            .await
            .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_quorum_expected_fail_when_not_met() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="quorum", quorum_count=2]
                p -> a
                p -> b
                p -> c
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("p").expect("node should exist");
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "success", "b": "fail", "c": "fail"}),
        );

        let outcome = NodeHandler::execute(&ParallelHandler::default(), node, &context, &graph)
            .await
            .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_wait_all_alias_expected_all_success_policy() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="wait_all"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("p").expect("node should exist");

        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &RuntimeContext::new(),
            &graph,
        )
        .await
        .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_first_success_alias_expected_any_success_policy() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="first_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("p").expect("node should exist");
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "success"}),
        );

        let outcome = NodeHandler::execute(&ParallelHandler::default(), node, &context, &graph)
            .await
            .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
    }
}
