//! Optional standard agent tools for `engine`.
//!
//! This crate owns optional tool packages, model-visible tool contracts,
//! and protocol/runtime adapters. The deterministic `engine` core stays
//! independent from this crate.

pub mod builtin;
pub mod concurrency;
pub mod environment;
pub mod environment_protocol;
pub mod error;
pub mod fleet;
pub mod fs;
pub mod limits;
pub mod prompts;
pub mod runtime;
pub mod skills;
pub mod toolset;
pub mod web;
pub mod workflow_tool;

pub use error::{ToolError, ToolResult};
