//! Tool execution SDK surface.
//!
//! The deterministic loop emits tool invocation effects. This module provides
//! the runner-facing contracts for resolving those effects to open tool
//! handlers without baking a specific async runtime into the core model.

pub mod dispatcher;
pub mod driver;
pub mod handler;

pub use dispatcher::{ToolDispatcher, ToolDispatcherBuilder, ToolDispatcherError};
pub use driver::{
    DispatchCall, DispatchCancellation, DispatchCancellationMode, DispatchCompletion,
    DispatchGroup, DispatchOutcome, DispatchRunRequest, InProcessToolDispatchDriver,
    PreparedToolDispatch, ToolDispatchDriver, ToolDispatchDriverError,
};
pub use handler::{
    ToolExecutionError, ToolHandler, ToolInvocationContext, ToolInvocationStatus,
    ToolResultMetadata, ToolResultStatus, ToolRuntimeHandle, ToolRuntimeSnapshot,
};
