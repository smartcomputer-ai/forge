use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub enum CodergenBackendResult {
    Text(String),
    Outcome(NodeOutcome),
}

#[async_trait]
pub trait CodergenBackend: Send + Sync {
    async fn run(
        &self,
        node: &Node,
        prompt: &str,
        context: &RuntimeContext,
    ) -> Result<CodergenBackendResult, AttractorError>;
}

#[derive(Debug, Default)]
pub struct NoopCodergenBackend;

#[async_trait]
impl CodergenBackend for NoopCodergenBackend {
    async fn run(
        &self,
        _node: &Node,
        _prompt: &str,
        _context: &RuntimeContext,
    ) -> Result<CodergenBackendResult, AttractorError> {
        Ok(CodergenBackendResult::Text(String::new()))
    }
}

pub struct CodergenHandler {
    backend: Option<Arc<dyn CodergenBackend>>,
}

impl CodergenHandler {
    pub fn new(backend: Option<Arc<dyn CodergenBackend>>) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl NodeHandler for CodergenHandler {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let mut prompt = node.attrs.get_str("prompt").unwrap_or_default().to_string();
        if prompt.trim().is_empty() {
            prompt = node
                .attrs
                .get_str("label")
                .unwrap_or(node.id.as_str())
                .to_string();
        }
        if let Some(goal) = graph.attrs.get_str("goal") {
            prompt = prompt.replace("$goal", goal);
        }

        if let Some(backend) = self.backend.as_ref() {
            match backend.run(node, &prompt, context).await {
                Ok(CodergenBackendResult::Outcome(outcome)) => return Ok(outcome),
                Ok(CodergenBackendResult::Text(response)) => {
                    return Ok(simulated_success(node, response));
                }
                Err(error) => return Ok(NodeOutcome::failure(error.to_string())),
            }
        }

        Ok(simulated_success(
            node,
            format!("[Simulated] Response for stage: {}", node.id),
        ))
    }
}

fn simulated_success(node: &Node, response_text: String) -> NodeOutcome {
    let mut updates = RuntimeContext::new();
    updates.insert("last_stage".to_string(), Value::String(node.id.clone()));
    updates.insert(
        "last_response".to_string(),
        Value::String(truncate(&response_text, 200)),
    );
    NodeOutcome {
        status: NodeStatus::Success,
        notes: Some(format!("Stage completed: {}", node.id)),
        context_updates: updates,
        preferred_label: None,
        suggested_next_ids: Vec::new(),
    }
}

fn truncate(input: &str, max_len: usize) -> String {
    input.chars().take(max_len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    struct RecordingBackend;

    #[async_trait]
    impl CodergenBackend for RecordingBackend {
        async fn run(
            &self,
            _node: &Node,
            prompt: &str,
            _context: &RuntimeContext,
        ) -> Result<CodergenBackendResult, AttractorError> {
            Ok(CodergenBackendResult::Text(format!("reply::{prompt}")))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn codergen_handler_expands_goal_and_returns_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [goal="ship"]
                n1 [shape=box, prompt="achieve $goal"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node should exist");
        let handler = CodergenHandler::new(Some(Arc::new(RecordingBackend)));
        let outcome = handler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome.context_updates.get("last_stage"),
            Some(&Value::String("n1".to_string()))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn codergen_handler_backend_outcome_expected_passthrough() {
        struct OutcomeBackend;
        #[async_trait]
        impl CodergenBackend for OutcomeBackend {
            async fn run(
                &self,
                _node: &Node,
                _prompt: &str,
                _context: &RuntimeContext,
            ) -> Result<CodergenBackendResult, AttractorError> {
                Ok(CodergenBackendResult::Outcome(NodeOutcome::failure(
                    "backend fail",
                )))
            }
        }

        let graph =
            parse_dot("digraph G { n1 [shape=box, label=\"x\"] }").expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node should exist");
        let handler = CodergenHandler::new(Some(Arc::new(OutcomeBackend)));
        let outcome = handler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }
}
