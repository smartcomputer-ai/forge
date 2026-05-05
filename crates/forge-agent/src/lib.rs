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

pub mod error;
pub mod r#loop;
pub mod model;
pub mod testing;

pub use r#loop::{decider, journal, planner, projection as loop_projection, reducer};
pub use model::{
    agent, batch, config, context, effects, events, ids, lifecycle, projection, refs, state,
    subagent, tooling, transcript, turn,
};
