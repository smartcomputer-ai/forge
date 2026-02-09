//! Error taxonomy and retry utilities.
//!
//! Implemented in P03.

use serde::{Deserialize, Serialize};

/// Minimal placeholder for stream/error typing. Expanded in P03.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SDKError {
    pub message: String,
}

impl SDKError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
