//! Core client and middleware system.
//!
//! Implemented in P05.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures::future::BoxFuture;

use crate::errors::{ConfigurationError, SDKError};
use crate::provider::{registered_factories, ProviderAdapter};
use crate::stream::StreamEventStream;
use crate::types::Request;
use crate::Response;

pub type CompleteHandler = Arc<dyn Fn(Request) -> BoxFuture<'static, Result<Response, SDKError>> + Send + Sync>;
pub type StreamHandler = Arc<dyn Fn(Request) -> BoxFuture<'static, Result<StreamEventStream, SDKError>> + Send + Sync>;

/// Middleware for wrapping complete() and stream() calls.
#[async_trait]
pub trait Middleware: Send + Sync {
    async fn handle_complete(&self, request: Request, next: CompleteHandler) -> Result<Response, SDKError>;

    async fn handle_stream(&self, request: Request, next: StreamHandler) -> Result<StreamEventStream, SDKError>;
}

#[derive(Clone, Default)]
pub struct Client {
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
    default_provider: Option<String>,
    middleware: Vec<Arc<dyn Middleware>>,
}

impl Client {
    pub fn new(
        providers: HashMap<String, Arc<dyn ProviderAdapter>>,
        default_provider: Option<String>,
        middleware: Vec<Arc<dyn Middleware>>,
    ) -> Self {
        Self {
            providers,
            default_provider,
            middleware,
        }
    }

    pub fn register_provider(&mut self, provider: Arc<dyn ProviderAdapter>) {
        let name = provider.name().to_string();
        if self.default_provider.is_none() {
            self.default_provider = Some(name.clone());
        }
        self.providers.insert(name, provider);
    }

    pub fn set_default_provider(&mut self, provider: impl Into<String>) {
        self.default_provider = Some(provider.into());
    }

    pub fn add_middleware(&mut self, middleware: Arc<dyn Middleware>) {
        self.middleware.push(middleware);
    }

    pub fn from_env() -> Result<Self, SDKError> {
        let mut providers = HashMap::new();
        let mut default_provider = None;

        for factory in registered_factories() {
            if let Some(adapter) = factory.from_env() {
                let name = adapter.name().to_string();
                if default_provider.is_none() {
                    default_provider = Some(name.clone());
                }
                providers.insert(name, adapter);
            }
        }

        Ok(Self {
            providers,
            default_provider,
            middleware: Vec::new(),
        })
    }

    pub async fn complete(&self, mut request: Request) -> Result<Response, SDKError> {
        let provider_name = self.resolve_provider(&request)?;
        request.provider = Some(provider_name.clone());
        let adapter = self
            .providers
            .get(&provider_name)
            .ok_or_else(|| SDKError::Configuration(ConfigurationError::new("provider not registered")))?
            .clone();

        let base: CompleteHandler = Arc::new(move |req| {
            let adapter = adapter.clone();
            Box::pin(async move { adapter.complete(req).await })
        });

        let handler = self
            .middleware
            .iter()
            .rev()
            .fold(base, |next, middleware| {
                let middleware = middleware.clone();
                Arc::new(move |req| {
                    let middleware = middleware.clone();
                    let next = next.clone();
                    Box::pin(async move { middleware.handle_complete(req, next).await })
                })
            });

        handler(request).await
    }

    pub async fn stream(&self, mut request: Request) -> Result<StreamEventStream, SDKError> {
        let provider_name = self.resolve_provider(&request)?;
        request.provider = Some(provider_name.clone());
        let adapter = self
            .providers
            .get(&provider_name)
            .ok_or_else(|| SDKError::Configuration(ConfigurationError::new("provider not registered")))?
            .clone();

        let base: StreamHandler = Arc::new(move |req| {
            let adapter = adapter.clone();
            Box::pin(async move { adapter.stream(req).await })
        });

        let handler = self
            .middleware
            .iter()
            .rev()
            .fold(base, |next, middleware| {
                let middleware = middleware.clone();
                Arc::new(move |req| {
                    let middleware = middleware.clone();
                    let next = next.clone();
                    Box::pin(async move { middleware.handle_stream(req, next).await })
                })
            });

        handler(request).await
    }

    fn resolve_provider(&self, request: &Request) -> Result<String, SDKError> {
        if let Some(provider) = &request.provider {
            return Ok(provider.clone());
        }
        if let Some(provider) = &self.default_provider {
            return Ok(provider.clone());
        }
        Err(SDKError::Configuration(ConfigurationError::new(
            "no provider configured",
        )))
    }
}

static DEFAULT_CLIENT: OnceLock<Client> = OnceLock::new();

/// Get the module-level default client, initializing from environment variables.
pub fn default_client() -> Result<&'static Client, SDKError> {
    if let Some(client) = DEFAULT_CLIENT.get() {
        return Ok(client);
    }

    let client = Client::from_env()?;
    if DEFAULT_CLIENT.set(client).is_err() {
        return DEFAULT_CLIENT
            .get()
            .ok_or_else(|| SDKError::Configuration(ConfigurationError::new("default client unavailable")));
    }

    DEFAULT_CLIENT
        .get()
        .ok_or_else(|| SDKError::Configuration(ConfigurationError::new("default client unavailable")))
}

/// Override the module-level default client.
pub fn set_default_client(client: Client) -> Result<(), SDKError> {
    DEFAULT_CLIENT
        .set(client)
        .map_err(|_| SDKError::Configuration(ConfigurationError::new("default client already set")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{register_provider_factory, ProviderFactory};
    use crate::stream::{StreamEvent, StreamEventStream, StreamEventType, StreamEventTypeOrString};
    use crate::types::{Message, Usage};
    use futures::stream;
    use std::sync::Mutex;

    struct TestAdapter {
        name: String,
    }

    #[async_trait]
    impl ProviderAdapter for TestAdapter {
        fn name(&self) -> &str {
            &self.name
        }

        async fn complete(&self, _request: Request) -> Result<Response, SDKError> {
            Ok(Response {
                id: "resp".to_string(),
                model: "model".to_string(),
                provider: self.name.clone(),
                message: Message::assistant("ok"),
                finish_reason: crate::types::FinishReason {
                    reason: "stop".to_string(),
                    raw: None,
                },
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                    total_tokens: 0,
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    raw: None,
                },
                raw: None,
                warnings: vec![],
                rate_limit: None,
            })
        }

        async fn stream(&self, _request: Request) -> Result<StreamEventStream, SDKError> {
            let event = StreamEvent {
                event_type: StreamEventTypeOrString::Known(StreamEventType::Finish),
                delta: None,
                text_id: None,
                reasoning_delta: None,
                tool_call: None,
                finish_reason: None,
                usage: None,
                response: None,
                error: None,
                raw: None,
            };
            Ok(Box::pin(stream::iter(vec![Ok(event)])))
        }
    }

    struct TestFactory;

    impl ProviderFactory for TestFactory {
        fn provider_id(&self) -> &'static str {
            "test"
        }

        fn from_env(&self) -> Option<Arc<dyn ProviderAdapter>> {
            if std::env::var("TEST_API_KEY").is_ok() {
                Some(Arc::new(TestAdapter {
                    name: "test".to_string(),
                }))
            } else {
                None
            }
        }
    }

    struct OrderMiddleware {
        label: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl Middleware for OrderMiddleware {
        async fn handle_complete(
            &self,
            request: Request,
            next: CompleteHandler,
        ) -> Result<Response, SDKError> {
            self.log.lock().unwrap().push(self.label);
            let result = next(request).await;
            self.log.lock().unwrap().push(self.label);
            result
        }

        async fn handle_stream(
            &self,
            request: Request,
            next: StreamHandler,
        ) -> Result<StreamEventStream, SDKError> {
            self.log.lock().unwrap().push(self.label);
            let result = next(request).await;
            self.log.lock().unwrap().push(self.label);
            result
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn middleware_order_is_preserved() {
        let adapter = Arc::new(TestAdapter {
            name: "test".to_string(),
        });
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("test".to_string(), adapter);
        let mut client = Client::new(providers, Some("test".to_string()), vec![]);

        let log = Arc::new(Mutex::new(Vec::new()));
        client.add_middleware(Arc::new(OrderMiddleware {
            label: "a",
            log: log.clone(),
        }));
        client.add_middleware(Arc::new(OrderMiddleware {
            label: "b",
            log: log.clone(),
        }));

        let request = Request {
            model: "model".to_string(),
            messages: vec![Message::user("hi")],
            provider: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort: None,
            metadata: None,
            provider_options: None,
        };

        let _ = client.complete(request).await.unwrap();
        let order = log.lock().unwrap().clone();
        assert_eq!(order, vec!["a", "b", "b", "a"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_resolution_prefers_request_provider() {
        let adapter = Arc::new(TestAdapter {
            name: "test".to_string(),
        });
        let mut client = Client::new(HashMap::new(), Some("test".to_string()), vec![]);
        client.register_provider(adapter);

        let request = Request {
            model: "model".to_string(),
            messages: vec![Message::user("hi")],
            provider: Some("test".to_string()),
            tools: None,
            tool_choice: None,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort: None,
            metadata: None,
            provider_options: None,
        };

        let response = client.complete(request).await.unwrap();
        assert_eq!(response.provider, "test");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn from_env_registers_provider_and_sets_default() {
        register_provider_factory(Arc::new(TestFactory));
        unsafe {
            std::env::set_var("TEST_API_KEY", "1");
        }

        let client = Client::from_env().unwrap();
        assert_eq!(client.default_provider.as_deref(), Some("test"));
        assert!(client.providers.contains_key("test"));

        unsafe {
            std::env::remove_var("TEST_API_KEY");
        }
    }
}
