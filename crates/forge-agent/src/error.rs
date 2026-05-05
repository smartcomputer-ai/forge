//! Core model error taxonomy.
//!
//! This module will contain state, lifecycle, validation, and future reducer
//! errors. Adapter-specific failures belong in effect receipts, not in the
//! deterministic model layer.

use thiserror::Error;

/// Errors raised by deterministic core-model helpers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelError {
    #[error("invalid lifecycle transition for {kind}: {from} -> {to}")]
    InvalidLifecycleTransition {
        kind: &'static str,
        from: String,
        to: String,
    },

    #[error("invalid model value for {field}: {message}")]
    InvalidValue {
        field: &'static str,
        message: String,
    },
}
