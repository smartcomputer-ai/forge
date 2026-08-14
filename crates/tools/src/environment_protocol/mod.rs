//! Environment protocol adapters.

pub mod conformance;
pub mod remote;

pub use conformance::{
    EnvironmentDataConformanceError, EnvironmentDataConformanceOptions,
    assert_environment_data_conformance,
};
pub use remote::{RemoteEnvironmentConnection, RemoteEnvironmentFileSystem, RemoteProcessExecutor};
