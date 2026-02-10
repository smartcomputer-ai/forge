use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;

#[derive(Debug, Default)]
pub struct ConditionalHandler;

#[async_trait]
impl NodeHandler for ConditionalHandler {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        Ok(NodeOutcome {
            status: NodeStatus::Success,
            notes: Some(format!("Conditional node evaluated: {}", node.id)),
            context_updates: RuntimeContext::new(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[tokio::test(flavor = "current_thread")]
    async fn conditional_handler_execute_expected_success() {
        let graph = parse_dot("digraph G { gate [shape=diamond] }").expect("graph should parse");
        let node = graph.nodes.get("gate").expect("gate node should exist");
        let outcome = ConditionalHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(
            outcome
                .notes
                .as_deref()
                .unwrap_or_default()
                .contains("gate")
        );
    }
}
