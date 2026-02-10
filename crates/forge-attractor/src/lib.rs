//! Attractor pipeline graph preparation library for Forge.
//!
//! This crate implements the spec/03 front-end pipeline:
//! parse DOT -> normalize to internal IR -> apply transforms -> validate.

pub mod artifacts;
pub mod backends;
pub mod checkpoint;
pub mod condition;
pub mod context;
pub mod diagnostics;
pub mod errors;
pub mod events;
pub mod fidelity;
pub mod graph;
pub mod handlers;
pub mod interviewer;
pub mod lint;
pub mod outcome;
pub mod parse;
pub mod resume;
pub mod retry;
pub mod routing;
pub mod runner;
pub mod runtime;
pub mod storage;
pub mod stylesheet;
pub mod transforms;

pub use artifacts::*;
pub use backends::*;
pub use checkpoint::*;
pub use condition::*;
pub use context::*;
pub use diagnostics::*;
pub use errors::*;
pub use events::*;
pub use fidelity::*;
pub use graph::*;
pub use handlers::*;
pub use interviewer::*;
pub use lint::*;
pub use parse::*;
pub use resume::*;
pub use retry::*;
pub use routing::*;
pub use runner::*;
pub use runtime::*;
pub use storage::*;
pub use stylesheet::*;
pub use transforms::*;
