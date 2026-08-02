//! Shared helpers for environment action tools.

use crate::error::ToolError;

pub(crate) fn invalid_request(message: impl Into<String>) -> ToolError {
    ToolError::InvalidRequest {
        message: message.into(),
    }
}

pub(crate) fn unsupported_capability(message: impl Into<String>) -> ToolError {
    ToolError::UnsupportedCapability {
        message: message.into(),
    }
}

pub(crate) fn unsupported_process_capability() -> ToolError {
    unsupported_capability(
        "environment_process_unavailable: the active environment does not expose process execution",
    )
}

pub(crate) fn unsupported_job_capability() -> ToolError {
    unsupported_capability(
        "environment_jobs_unavailable: the active environment does not expose durable jobs",
    )
}
