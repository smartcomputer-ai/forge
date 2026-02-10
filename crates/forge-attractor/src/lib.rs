//! Attractor pipeline graph preparation library for Forge.
//!
//! This crate implements the spec/03 front-end pipeline:
//! parse DOT -> normalize to internal IR -> apply transforms -> validate.

pub mod backends;
pub mod artifacts;
pub mod checkpoint;
pub mod condition;
pub mod context;
pub mod diagnostics;
pub mod errors;
pub mod graph;
pub mod handlers;
pub mod lint;
pub mod outcome;
pub mod parse;
pub mod retry;
pub mod routing;
pub mod resume;
pub mod runner;
pub mod runtime;
pub mod storage;
pub mod stylesheet;
pub mod transforms;

pub use backends::*;
pub use artifacts::*;
pub use checkpoint::*;
pub use condition::*;
pub use context::*;
pub use diagnostics::*;
pub use errors::*;
pub use graph::*;
pub use handlers::*;
pub use lint::*;
pub use parse::*;
pub use retry::*;
pub use routing::*;
pub use resume::*;
pub use runner::*;
pub use runtime::*;
pub use storage::*;
pub use stylesheet::*;
pub use transforms::*;
