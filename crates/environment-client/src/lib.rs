//! Typed client for environment protocol data-plane and controller-plane calls.
//!
//! The crate owns transport and JSON-RPC mechanics while reusing pure protocol
//! records from `environment-protocol`.

pub mod control;
pub mod data;
pub mod error;
pub mod rpc;
pub mod transport;

pub use control::EnvironmentProviderClient;
pub use data::EnvironmentDataClient;
pub use error::{EnvironmentClientError, EnvironmentClientResult};
pub use rpc::{JsonRpcClient, JsonRpcNotification, JsonRpcTransport};
pub use transport::{WebSocketConnectOptions, WebSocketTransport};
