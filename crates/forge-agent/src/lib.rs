//! Coding agent loop library for Forge.
//!
//! This crate is the implementation target for `spec/02-coding-agent-loop-spec.md`.
//! The initial milestone establishes module boundaries and public types for:
//! session orchestration, provider profiles, tools, execution environments,
//! event delivery, and output truncation.

pub mod config;
pub mod errors;
pub mod events;
pub mod execution;
pub mod profiles;
pub mod session;
pub mod tools;
pub mod truncation;
pub mod turn;

pub use config::*;
pub use errors::*;
pub use events::*;
pub use execution::*;
pub use profiles::*;
pub use session::*;
pub use tools::*;
pub use truncation::*;
pub use turn::*;
