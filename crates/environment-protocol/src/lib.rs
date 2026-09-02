//! Protocol types for environment execution targets.
//!
//! This crate is intentionally transport-free. It defines the stable
//! request/response records used by clients, provider controllers, and environment
//! implementations.

pub mod control;
pub mod data;
pub mod error;
pub mod gateway;
pub mod registration;
pub mod shared;
