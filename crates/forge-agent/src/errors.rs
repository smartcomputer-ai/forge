use thiserror::Error;

/// Session-level failures in orchestration and lifecycle management.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SessionError {
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("session is closed")]
    Closed,
    #[error("invalid session state transition: {from} -> {to}")]
    InvalidStateTransition { from: String, to: String },
    #[error("event payload serialization failed: {0}")]
    EventSerialization(String),
    #[error("checkpoint not supported: {0}")]
    CheckpointUnsupported(String),
    #[error("turnstore persistence failed: {0}")]
    Persistence(String),
}

/// Tool-level failures in lookup, validation, and execution.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("tool validation failed: {0}")]
    Validation(String),
    #[error("tool execution failed: {0}")]
    Execution(String),
}

/// Top-level error type for the forge-agent crate.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("execution environment error: {0}")]
    ExecutionEnvironment(String),
    #[error("not implemented yet: {0}")]
    NotImplemented(String),
    #[error(transparent)]
    Llm(#[from] forge_llm::SDKError),
}

impl AgentError {
    pub fn session_closed() -> Self {
        Self::Session(SessionError::Closed)
    }
}
