//! Provider adapter contract.
//!
//! Implemented in P05+.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;

use crate::errors::SDKError;
use crate::stream::StreamEventStream;
use crate::types::{Request, Response};

/// Provider adapter contract.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(&self, request: Request) -> Result<Response, SDKError>;

    async fn stream(&self, request: Request) -> Result<StreamEventStream, SDKError>;
}

/// Factory for building adapters from environment variables.
pub trait ProviderFactory: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn from_env(&self) -> Option<Arc<dyn ProviderAdapter>>;
}

static PROVIDER_FACTORIES: OnceLock<Mutex<HashMap<&'static str, Arc<dyn ProviderFactory>>>> =
    OnceLock::new();

fn factories() -> &'static Mutex<HashMap<&'static str, Arc<dyn ProviderFactory>>> {
    PROVIDER_FACTORIES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a provider factory for Client::from_env().
///
/// Provider adapter crates should call this during initialization.
pub fn register_provider_factory(factory: Arc<dyn ProviderFactory>) {
    let mut registry = factories().lock().expect("provider factory registry");
    registry.insert(factory.provider_id(), factory);
}

/// Get a snapshot of registered factories.
pub fn registered_factories() -> Vec<Arc<dyn ProviderFactory>> {
    let registry = factories().lock().expect("provider factory registry");
    registry.values().cloned().collect()
}
