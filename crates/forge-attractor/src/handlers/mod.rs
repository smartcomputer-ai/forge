use crate::{AttractorError, Graph, Node, NodeOutcome, RuntimeContext};
use async_trait::async_trait;
use std::sync::Arc;

pub mod codergen;
pub mod conditional;
pub mod exit;
pub mod registry;
pub mod start;
pub mod tool;
pub mod wait_human;

#[async_trait]
pub trait NodeHandler: Send + Sync {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError>;
}

pub type SharedNodeHandler = Arc<dyn NodeHandler>;

#[async_trait]
impl<T> crate::NodeExecutor for T
where
    T: NodeHandler + Send + Sync,
{
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        NodeHandler::execute(self, node, context, graph).await
    }
}

pub fn core_registry() -> registry::HandlerRegistry {
    let mut registry = registry::HandlerRegistry::new();
    registry.register_type("start", Arc::new(start::StartHandler));
    registry.register_type("exit", Arc::new(exit::ExitHandler));
    registry.register_type("codergen", Arc::new(codergen::CodergenHandler::new(None)));
    registry.register_type("conditional", Arc::new(conditional::ConditionalHandler));
    registry.register_type(
        "wait.human",
        Arc::new(wait_human::WaitHumanHandler::new(Arc::new(
            wait_human::AutoApproveInterviewer,
        ))),
    );
    registry.register_type("tool", Arc::new(tool::ToolHandler));
    registry
}
