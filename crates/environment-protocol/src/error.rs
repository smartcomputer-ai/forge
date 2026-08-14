//! Shared error payloads for environment protocol methods.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentProtocolError {
    pub code: EnvironmentProtocolErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl EnvironmentProtocolError {
    pub fn new(code: EnvironmentProtocolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentProtocolErrorCode {
    InvalidRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unsupported,
    CapabilityUnavailable,
    ProcessFailed,
    Timeout,
    TransportClosed,
    Internal,
}
