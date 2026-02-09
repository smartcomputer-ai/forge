use crate::{AgentError, ExecutionEnvironment};
use forge_llm::ToolDefinition;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, AgentError>> + Send>>;
pub type ToolExecutor =
    Arc<dyn Fn(Value, Arc<dyn ExecutionEnvironment>) -> ToolFuture + Send + Sync>;

#[derive(Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    pub executor: ToolExecutor,
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: RegisteredTool) {
        self.tools.insert(tool.definition.name.clone(), tool);
    }

    pub fn unregister(&mut self, name: &str) -> Option<RegisteredTool> {
        self.tools.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| tool.definition.clone())
            .collect()
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort_unstable();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_executor() -> ToolExecutor {
        Arc::new(|_args, _env| Box::pin(async move { Ok("ok".to_string()) }))
    }

    #[test]
    fn tool_registry_latest_registration_wins() {
        let mut registry = ToolRegistry::default();

        let first = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "first".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(first);

        let second = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "second".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(second);

        let registered = registry
            .get("read_file")
            .expect("tool should be present after replacement");
        assert_eq!(registered.definition.description, "second");
    }
}
