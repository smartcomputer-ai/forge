use thiserror::Error;

/// Top-level error type for the forge-agent crate.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("session is closed")]
    SessionClosed,
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool validation failed: {0}")]
    ToolValidation(String),
    #[error("execution environment error: {0}")]
    Execution(String),
    #[error("not implemented yet: {0}")]
    NotImplemented(String),
    #[error(transparent)]
    Llm(#[from] forge_llm::SDKError),
}
