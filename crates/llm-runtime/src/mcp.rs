use async_trait::async_trait;
use engine::RemoteMcpToolSpec;
use serde_json::Value;

pub const MAX_NATIVE_MCP_TOOLS_PER_REQUEST: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct NativeMcpTool {
    pub remote_name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct McpInventoryError {
    pub message: String,
}

impl McpInventoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait McpInventoryResolver: Send + Sync {
    async fn list_tools(
        &self,
        spec: &RemoteMcpToolSpec,
    ) -> Result<Vec<NativeMcpTool>, McpInventoryError>;
}

#[derive(Default)]
pub struct UnconfiguredMcpInventoryResolver;

#[async_trait]
impl McpInventoryResolver for UnconfiguredMcpInventoryResolver {
    async fn list_tools(
        &self,
        spec: &RemoteMcpToolSpec,
    ) -> Result<Vec<NativeMcpTool>, McpInventoryError> {
        Err(McpInventoryError::new(format!(
            "native MCP inventory resolver is not configured for {}",
            spec.server_id
        )))
    }
}
