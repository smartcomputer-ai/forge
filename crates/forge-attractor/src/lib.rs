//! Attractor pipeline graph preparation library for Forge.
//!
//! This crate implements the spec/03 front-end pipeline:
//! parse DOT -> normalize to internal IR -> apply transforms -> validate.

pub mod diagnostics;
pub mod errors;
pub mod graph;
pub mod lint;
pub mod parse;
pub mod storage;
pub mod stylesheet;
pub mod transforms;

pub use diagnostics::*;
pub use errors::*;
pub use graph::*;
pub use lint::*;
pub use parse::*;
pub use storage::*;
pub use stylesheet::*;
pub use transforms::*;
