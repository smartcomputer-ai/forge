use crate::Node;
use crate::handlers::SharedNodeHandler;
use crate::{AttractorError, Graph, NodeOutcome, RuntimeContext};
use std::collections::BTreeMap;

const DEFAULT_HANDLER_TYPE: &str = "codergen";

#[derive(Default)]
pub struct HandlerRegistry {
    handlers_by_type: BTreeMap<String, SharedNodeHandler>,
    shape_to_type: BTreeMap<String, String>,
    default_handler_type: String,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers_by_type: BTreeMap::new(),
            shape_to_type: default_shape_mapping(),
            default_handler_type: DEFAULT_HANDLER_TYPE.to_string(),
        }
    }

    pub fn register_type(
        &mut self,
        handler_type: impl Into<String>,
        handler: SharedNodeHandler,
    ) -> Option<SharedNodeHandler> {
        self.handlers_by_type.insert(handler_type.into(), handler)
    }

    pub fn register_shape_mapping(
        &mut self,
        shape: impl Into<String>,
        handler_type: impl Into<String>,
    ) -> Option<String> {
        self.shape_to_type.insert(shape.into(), handler_type.into())
    }

    pub fn set_default_handler_type(&mut self, handler_type: impl Into<String>) {
        self.default_handler_type = handler_type.into();
    }

    pub fn resolve_handler_type(&self, node: &Node) -> String {
        if let Some(node_type) = node.attrs.get_str("type") {
            let trimmed = node_type.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }

        let shape = node.attrs.get_str("shape").unwrap_or("box");
        self.shape_to_type
            .get(shape)
            .cloned()
            .unwrap_or_else(|| self.default_handler_type.clone())
    }

    pub fn resolve_handler(&self, node: &Node) -> Option<SharedNodeHandler> {
        let handler_type = self.resolve_handler_type(node);
        self.handlers_by_type
            .get(&handler_type)
            .cloned()
            .or_else(|| {
                self.handlers_by_type
                    .get(&self.default_handler_type)
                    .cloned()
            })
    }
}

pub fn resolve_handler_type_from_node(node: &Node) -> String {
    HandlerRegistry::new().resolve_handler_type(node)
}

pub struct RegistryNodeExecutor {
    pub registry: HandlerRegistry,
}

impl RegistryNodeExecutor {
    pub fn new(registry: HandlerRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl crate::NodeExecutor for RegistryNodeExecutor {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let handler = self.registry.resolve_handler(node).ok_or_else(|| {
            AttractorError::Runtime(format!(
                "no handler registered for type '{}'",
                self.registry.resolve_handler_type(node)
            ))
        })?;
        handler.execute(node, context, graph).await
    }
}

fn default_shape_mapping() -> BTreeMap<String, String> {
    [
        ("Mdiamond", "start"),
        ("Msquare", "exit"),
        ("box", "codergen"),
        ("hexagon", "wait.human"),
        ("diamond", "conditional"),
        ("component", "parallel"),
        ("tripleoctagon", "parallel.fan_in"),
        ("parallelogram", "tool"),
        ("house", "stack.manager_loop"),
    ]
    .into_iter()
    .map(|(shape, handler_type)| (shape.to_string(), handler_type.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttractorError, Graph, Node, NodeExecutor, NodeOutcome, RuntimeContext, parse_dot};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct SuccessHandler;

    #[async_trait]
    impl crate::handlers::NodeHandler for SuccessHandler {
        async fn execute(
            &self,
            _node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            Ok(NodeOutcome::success())
        }
    }

    fn node_with_attrs(attrs: &str) -> Node {
        let graph =
            parse_dot(&format!("digraph G {{ n1 [{attrs}] }}")).expect("graph should parse");
        graph.nodes.get("n1").expect("node should exist").clone()
    }

    #[test]
    fn resolve_handler_type_explicit_type_expected_highest_precedence() {
        let registry = HandlerRegistry::new();
        let node = node_with_attrs("shape=diamond, type=\"tool\"");
        assert_eq!(registry.resolve_handler_type(&node), "tool");
    }

    #[test]
    fn resolve_handler_type_shape_mapping_expected_used_when_type_absent() {
        let registry = HandlerRegistry::new();
        let node = node_with_attrs("shape=hexagon");
        assert_eq!(registry.resolve_handler_type(&node), "wait.human");
    }

    #[test]
    fn resolve_handler_type_unknown_shape_expected_default_handler_type() {
        let registry = HandlerRegistry::new();
        let node = node_with_attrs("shape=unknown");
        assert_eq!(registry.resolve_handler_type(&node), "codergen");
    }

    #[test]
    fn resolve_handler_unregistered_explicit_type_expected_default_handler_instance() {
        let mut registry = HandlerRegistry::new();
        let default_handler: Arc<dyn crate::handlers::NodeHandler> = Arc::new(SuccessHandler);
        registry.register_type("codergen", default_handler.clone());

        let node = node_with_attrs("type=\"custom.handler\"");
        let resolved = registry
            .resolve_handler(&node)
            .expect("default handler should be returned");

        assert!(Arc::ptr_eq(&resolved, &default_handler));
    }

    #[test]
    fn resolve_handler_registered_explicit_type_expected_specific_handler_instance() {
        let mut registry = HandlerRegistry::new();
        let default_handler: Arc<dyn crate::handlers::NodeHandler> = Arc::new(SuccessHandler);
        let tool_handler: Arc<dyn crate::handlers::NodeHandler> = Arc::new(SuccessHandler);
        registry.register_type("codergen", default_handler);
        registry.register_type("tool", tool_handler.clone());

        let node = node_with_attrs("shape=box, type=\"tool\"");
        let resolved = registry
            .resolve_handler(&node)
            .expect("tool handler should be resolved");

        assert!(Arc::ptr_eq(&resolved, &tool_handler));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registry_node_executor_unregistered_type_without_default_expected_error() {
        let mut registry = HandlerRegistry::new();
        registry.set_default_handler_type("missing.default");
        let graph = parse_dot("digraph G { n1 [type=\"custom\"] }").expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node should exist");
        let executor = RegistryNodeExecutor::new(registry);

        let error = executor
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect_err("execution should fail");
        assert!(matches!(error, AttractorError::Runtime(_)));
    }
}
