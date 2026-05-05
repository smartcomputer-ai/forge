//! Forge-native coding agent runtime.
//!
//! This crate targets [`spec/04-new-agent-spec.md`](../../spec/04-new-agent-spec.md).
//! The first implementation phase establishes durable model contracts for
//! agent definitions, scoped journal events, effect intents/receipts, refs,
//! bounded session state, transcript projections, and planning snapshots.
//!
//! The core model is designed to stay deterministic and runner-agnostic. Local
//! execution, Temporal workflows, LLM providers, host tools, CXDB persistence,
//! and CLI projections are adapter or runner concerns layered around these
//! contracts. Hook, policy, approval, and sandbox APIs are deferred extension
//! surfaces rather than supported first-cut behavior.

pub mod agent;
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
pub mod transcript;
pub mod turn;
