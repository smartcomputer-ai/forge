//! Client-side environment protocol errors.

use environment_protocol::error::EnvironmentProtocolError;
use thiserror::Error;

pub type EnvironmentClientResult<T> = Result<T, EnvironmentClientError>;

#[derive(Debug, Error)]
pub enum EnvironmentClientError {
    #[error("failed to serialize JSON-RPC message: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("transport closed")]
    TransportClosed,

    #[error("transport error: {0}")]
    Transport(String),

    #[error("invalid JSON-RPC message: {0}")]
    InvalidMessage(String),

    #[error("environment protocol error: {0:?}")]
    Protocol(EnvironmentProtocolError),
}
