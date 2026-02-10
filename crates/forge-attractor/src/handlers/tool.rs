use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug, Default)]
pub struct ToolHandler;

#[async_trait]
impl NodeHandler for ToolHandler {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let command = node
            .attrs
            .get_str("tool_command")
            .unwrap_or_default()
            .trim();
        if command.is_empty() {
            return Ok(NodeOutcome::failure("No tool_command specified"));
        }

        let output = node
            .attrs
            .get_str("tool_output")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("[Simulated tool output] {command}"));
        let mut updates = RuntimeContext::new();
        updates.insert("tool.output".to_string(), Value::String(output.clone()));

        Ok(NodeOutcome {
            status: NodeStatus::Success,
            notes: Some(format!("Tool completed: {command}")),
            context_updates: updates,
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
    async fn tool_handler_missing_command_expected_fail() {
        let graph = parse_dot("digraph G { t [shape=parallelogram] }").expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_command_expected_success_and_output_update() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="echo hi"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(outcome.context_updates.contains_key("tool.output"));
    }
}
