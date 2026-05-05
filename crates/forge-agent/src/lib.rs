//! Forge-native coding agent runtime.
//!
//! This crate targets [`spec/04-new-agent-spec.md`](../../spec/04-new-agent-spec.md).
//! The first implementation phase establishes the public core-model module
//! boundaries only. Later roadmap phases fill these modules with the durable
//! state, event, effect, context, tool, and projection contracts.
//!
//! The core model is designed to stay deterministic and runner-agnostic. Local
//! execution, Temporal workflows, LLM providers, host tools, CXDB persistence,
//! and CLI projections are adapter or runner concerns layered around these
//! contracts.

pub mod batch;
pub mod config;
pub mod context;
pub mod effects;
pub mod error;
pub mod events;
pub mod ids;
pub mod lifecycle;
pub mod projection;
pub mod refs;
pub mod state;
pub mod subagent;
pub mod tooling;
pub mod trace;
pub mod transcript;
pub mod turn;
