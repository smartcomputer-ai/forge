use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use auth::{
    AuthGrantStore, AuthProviderKind, AuthProviderStore, AuthTokenBroker, GitHubAppRuntime,
    GrantRefreshLock, OAuthClientStore, OAuthRefreshRuntime, RegistryTokenBroker, SecretStore,
};
use futures_util::future::join_all;
use llm_clients::{LlmApiError, anthropic::messages as anthropic, openai::responses as openai};
use llm_runtime::{ModelProviderResolver, provider_keys::ProviderKeyError};
use store_pg::PgStore;

use super::*;

const OPENAI_PROVIDER_ID: &str = "openai";
const ANTHROPIC_PROVIDER_ID: &str = "anthropic";
const OPENAI_RESPONSES_API_KIND: &str = "openai:responses";
const OPENAI_COMPLETIONS_API_KIND: &str = "openai:completions";
const ANTHROPIC_MESSAGES_API_KIND: &str = "anthropic:messages";
const MODEL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
/// Keep the normal picker focused on roughly the last eighteen months of
/// OpenAI models. Older account-visible ids remain usable through manual model
/// entry and through models/list with selectableOnly=false.
const OPENAI_SELECTABLE_MAX_AGE_MS: i64 = 548 * 24 * 60 * 60 * 1_000;

/// The whole P97 route set. This remains code-local deliberately: provider
/// discovery is direct, not a registry or persisted catalog.
pub(super) struct ModelDiscoveryService {
    openai: Arc<openai::Client>,
    anthropic: Arc<anthropic::Client>,
    provider_keys: Arc<dyn ModelProviderResolver>,
    providers: Arc<dyn AuthProviderStore>,
}

impl ModelDiscoveryService {
    pub(super) fn new(
        openai: Arc<openai::Client>,
        anthropic: Arc<anthropic::Client>,
        provider_keys: Arc<dyn ModelProviderResolver>,
        providers: Arc<dyn AuthProviderStore>,
    ) -> Self {
        Self {
            openai,
            anthropic,
            provider_keys,
            providers,
        }
    }

    pub(super) async fn list(&self, selectable_only: bool) -> ModelListResponse {
        let (openai, anthropic, custom) = tokio::join!(
            self.list_openai(),
            self.list_anthropic(),
            self.list_custom_providers()
        );
        let ((mut models, provider_a), (models_b, provider_b), (models_c, providers_c)) =
            (openai, anthropic, custom);
        models.extend(models_b);
        models.extend(models_c);
        let mut providers = vec![provider_a, provider_b];
        providers.extend(providers_c);
        models.sort_by(|left, right| {
            (
                &left.provider_id,
                &left.api_kind,
                &left.display_name,
                &left.model,
            )
                .cmp(&(
                    &right.provider_id,
                    &right.api_kind,
                    &right.display_name,
                    &right.model,
                ))
        });
        if selectable_only {
            models.retain(|model| {
                model.provider_id != OPENAI_PROVIDER_ID
                    || (is_openai_selectable_model(&model.model)
                        && is_openai_recent_model(model.created_at_ms, model.fetched_at_ms))
            });
        }
        ModelListResponse { models, providers }
    }

    async fn list_openai(&self) -> (Vec<ModelView>, ModelProviderDiscoveryView) {
        let mut source = ModelProviderCredentialSource::Deployment;
        let mut credential = ModelProviderCredentialStatus::Configured;
        let result = tokio::time::timeout(MODEL_DISCOVERY_TIMEOUT, async {
            let provider = self
                .provider_keys
                .resolve_model_provider(OPENAI_PROVIDER_ID)
                .await
                .map_err(DiscoveryError::ProviderKey)?;
            if provider.is_some() {
                source = ModelProviderCredentialSource::Universe;
                if provider
                    .as_ref()
                    .is_some_and(|provider| provider.auth.is_none())
                {
                    credential = ModelProviderCredentialStatus::NotRequired;
                }
            }
            self.openai
                .list_models_with_transport(
                    provider.as_ref().map(|provider| provider.as_request_auth()),
                    provider
                        .as_ref()
                        .and_then(|provider| provider.endpoint.as_ref())
                        .map(|endpoint| &endpoint.transport),
                )
                .await
                .map_err(DiscoveryError::Provider)
        })
        .await
        .map_err(|_| DiscoveryError::Timeout)
        .and_then(|result| result);
        match result {
            Ok(response) => {
                let fetched_at_ms = discovery_now_ms();
                let models = response
                    .parsed
                    .data
                    .into_iter()
                    .flat_map(|model| openai_model_views(model, fetched_at_ms))
                    .collect();
                (
                    models,
                    provider_success(
                        OPENAI_PROVIDER_ID,
                        &[OPENAI_RESPONSES_API_KIND, OPENAI_COMPLETIONS_API_KIND],
                        fetched_at_ms,
                        source,
                        credential,
                    ),
                )
            }
            Err(error) => (
                Vec::new(),
                provider_failure(
                    OPENAI_PROVIDER_ID,
                    &[OPENAI_RESPONSES_API_KIND, OPENAI_COMPLETIONS_API_KIND],
                    &error,
                    source,
                ),
            ),
        }
    }

    async fn list_anthropic(&self) -> (Vec<ModelView>, ModelProviderDiscoveryView) {
        let mut source = ModelProviderCredentialSource::Deployment;
        let mut credential = ModelProviderCredentialStatus::Configured;
        let result = tokio::time::timeout(MODEL_DISCOVERY_TIMEOUT, async {
            let provider = self
                .provider_keys
                .resolve_model_provider(ANTHROPIC_PROVIDER_ID)
                .await
                .map_err(DiscoveryError::ProviderKey)?;
            if provider.is_some() {
                source = ModelProviderCredentialSource::Universe;
                if provider
                    .as_ref()
                    .is_some_and(|provider| provider.auth.is_none())
                {
                    credential = ModelProviderCredentialStatus::NotRequired;
                }
            }
            self.anthropic
                .list_models_with_auth(provider.as_ref().map(|provider| provider.as_request_auth()))
                .await
                .map_err(DiscoveryError::Provider)
        })
        .await
        .map_err(|_| DiscoveryError::Timeout)
        .and_then(|result| result);
        match result {
            Ok(models) => {
                let fetched_at_ms = discovery_now_ms();
                let models = models
                    .into_iter()
                    .map(|model| ModelView {
                        provider_id: ANTHROPIC_PROVIDER_ID.to_owned(),
                        api_kind: ANTHROPIC_MESSAGES_API_KIND.to_owned(),
                        display_name: model.display_name.unwrap_or_else(|| model.id.clone()),
                        capabilities: ModelCapabilitiesView {
                            reasoning_efforts: anthropic_reasoning_efforts(
                                model.capabilities.as_ref(),
                            ),
                            parallel_tool_use: None,
                            max_output_tokens: model.max_tokens,
                            max_input_tokens: model.max_input_tokens,
                        },
                        model: model.id,
                        created_at_ms: None,
                        source: ModelSource::Provider,
                        fetched_at_ms,
                    })
                    .collect();
                (
                    models,
                    provider_success(
                        ANTHROPIC_PROVIDER_ID,
                        &[ANTHROPIC_MESSAGES_API_KIND],
                        fetched_at_ms,
                        source,
                        credential,
                    ),
                )
            }
            Err(error) => (
                Vec::new(),
                provider_failure(
                    ANTHROPIC_PROVIDER_ID,
                    &[ANTHROPIC_MESSAGES_API_KIND],
                    &error,
                    source,
                ),
            ),
        }
    }

    async fn list_custom_providers(&self) -> (Vec<ModelView>, Vec<ModelProviderDiscoveryView>) {
        let records = match self.providers.list_auth_providers().await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(error = %error, "list universe model providers for discovery");
                return (Vec::new(), Vec::new());
            }
        };
        let providers = records.into_iter().filter_map(|record| {
            let provider_id = record
                .provider_id
                .as_str()
                .strip_prefix("model:")?
                .to_owned();
            if matches!(
                provider_id.as_str(),
                OPENAI_PROVIDER_ID | ANTHROPIC_PROVIDER_ID
            ) {
                return None;
            }
            let endpoint = model_endpoint(&record.config)?;
            Some((
                provider_id,
                endpoint.api_kinds.clone(),
                matches!(record.config, auth::AuthProviderConfig::ModelEndpoint(_)),
            ))
        });
        let results = join_all(
            providers.map(|(provider_id, api_kinds, anonymous)| async move {
                self.list_custom_provider(provider_id, api_kinds, anonymous)
                    .await
            }),
        )
        .await;
        let mut models = Vec::new();
        let mut statuses = Vec::new();
        for (provider_models, status) in results {
            models.extend(provider_models);
            statuses.push(status);
        }
        (models, statuses)
    }

    async fn list_custom_provider(
        &self,
        provider_id: String,
        api_kinds: Vec<String>,
        anonymous: bool,
    ) -> (Vec<ModelView>, ModelProviderDiscoveryView) {
        let result = tokio::time::timeout(MODEL_DISCOVERY_TIMEOUT, async {
            let provider = self
                .provider_keys
                .resolve_model_provider(&provider_id)
                .await
                .map_err(DiscoveryError::ProviderKey)?
                .ok_or_else(|| {
                    DiscoveryError::ProviderKey(ProviderKeyError::NotUsable {
                        provider_id: provider_id.clone(),
                        message: "provider row disappeared during discovery".to_owned(),
                    })
                })?;
            let endpoint = provider.endpoint.as_ref().ok_or_else(|| {
                DiscoveryError::ProviderKey(ProviderKeyError::NotUsable {
                    provider_id: provider_id.clone(),
                    message: "custom provider has no endpoint".to_owned(),
                })
            })?;
            self.openai
                .list_models_with_transport(
                    Some(provider.as_request_auth()),
                    Some(&endpoint.transport),
                )
                .await
                .map_err(DiscoveryError::Provider)
        })
        .await
        .map_err(|_| DiscoveryError::Timeout)
        .and_then(|result| result);
        match result {
            Ok(response) => {
                let fetched_at_ms = discovery_now_ms();
                let models = response
                    .parsed
                    .data
                    .into_iter()
                    .flat_map(|model| {
                        let provider_id = provider_id.clone();
                        let created_at_ms = unix_seconds_to_millis(model.created);
                        api_kinds.iter().map(move |api_kind| ModelView {
                            provider_id: provider_id.clone(),
                            api_kind: api_kind.clone(),
                            display_name: model.id.clone(),
                            model: model.id.clone(),
                            capabilities: ModelCapabilitiesView::default(),
                            created_at_ms,
                            source: ModelSource::Provider,
                            fetched_at_ms,
                        })
                    })
                    .collect();
                (
                    models,
                    provider_success(
                        &provider_id,
                        &api_kinds.iter().map(String::as_str).collect::<Vec<_>>(),
                        fetched_at_ms,
                        ModelProviderCredentialSource::Universe,
                        if anonymous {
                            ModelProviderCredentialStatus::NotRequired
                        } else {
                            ModelProviderCredentialStatus::Configured
                        },
                    ),
                )
            }
            Err(error) => (
                Vec::new(),
                provider_failure(
                    &provider_id,
                    &api_kinds.iter().map(String::as_str).collect::<Vec<_>>(),
                    &error,
                    ModelProviderCredentialSource::Universe,
                ),
            ),
        }
    }
}

fn model_endpoint(config: &auth::AuthProviderConfig) -> Option<&auth::ModelEndpointConfig> {
    match config {
        auth::AuthProviderConfig::ModelApiKey(config) => config.endpoint.as_ref(),
        auth::AuthProviderConfig::ModelOAuth(config) => config.endpoint.as_ref(),
        auth::AuthProviderConfig::ModelEndpoint(config) => Some(&config.endpoint),
        auth::AuthProviderConfig::GitHubApp(_) => None,
    }
}

fn openai_model_views(model: openai::Model, fetched_at_ms: i64) -> [ModelView; 2] {
    let created_at_ms = unix_seconds_to_millis(model.created);
    let capabilities = openai_model_capabilities(&model.id);
    [OPENAI_RESPONSES_API_KIND, OPENAI_COMPLETIONS_API_KIND].map(|api_kind| ModelView {
        provider_id: OPENAI_PROVIDER_ID.to_owned(),
        api_kind: api_kind.to_owned(),
        display_name: model.id.clone(),
        model: model.id.clone(),
        capabilities: capabilities.clone(),
        created_at_ms,
        source: ModelSource::Provider,
        fetched_at_ms,
    })
}

fn unix_seconds_to_millis(seconds: Option<i64>) -> Option<i64> {
    seconds?.checked_mul(1_000)
}

/// OpenAI's Models API reports identity and creation time, but not reasoning
/// capabilities. Keep this deliberately small and family-based so aliases and
/// dated snapshots receive the same officially documented effort vocabulary.
fn openai_model_capabilities(model: &str) -> ModelCapabilitiesView {
    const GPT_5_6: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
    const GPT_5_2_TO_5_5: &[&str] = &["none", "low", "medium", "high", "xhigh"];
    const GPT_5_PRO: &[&str] = &["medium", "high", "xhigh"];
    const GPT_5_1: &[&str] = &["none", "low", "medium", "high"];
    const GPT_5: &[&str] = &["minimal", "low", "medium", "high"];

    let efforts = if ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]
        .iter()
        .any(|family| is_model_family(model, family))
    {
        Some(GPT_5_6)
    } else if ["gpt-5.5-pro", "gpt-5.4-pro", "gpt-5.2-pro"]
        .iter()
        .any(|family| is_model_family(model, family))
    {
        Some(GPT_5_PRO)
    } else if [
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.4-nano",
        "gpt-5.2",
    ]
    .iter()
    .any(|family| is_model_family(model, family))
    {
        Some(GPT_5_2_TO_5_5)
    } else if is_model_family(model, "gpt-5-pro") {
        Some(&["high"][..])
    } else if is_model_family(model, "gpt-5.1") {
        Some(GPT_5_1)
    } else if ["gpt-5", "gpt-5-mini", "gpt-5-nano"]
        .iter()
        .any(|family| is_model_family(model, family))
    {
        Some(GPT_5)
    } else {
        None
    };

    ModelCapabilitiesView {
        reasoning_efforts: efforts
            .map(|efforts| efforts.iter().map(|effort| (*effort).to_owned()).collect()),
        ..Default::default()
    }
}

fn is_model_family(model: &str, family: &str) -> bool {
    model == family
        || model
            .strip_prefix(family)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(is_date_snapshot)
}

fn is_date_snapshot(suffix: &str) -> bool {
    suffix.len() == 10
        && suffix.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7)
                .then_some(byte == b'-')
                .unwrap_or(byte.is_ascii_digit())
        })
}

fn is_openai_recent_model(created_at_ms: Option<i64>, fetched_at_ms: i64) -> bool {
    created_at_ms.is_none_or(|created_at_ms| {
        fetched_at_ms.saturating_sub(created_at_ms) <= OPENAI_SELECTABLE_MAX_AGE_MS
    })
}

/// `GET /v1/models` does not say which endpoint a model supports. This is
/// intentionally only a small exclusion policy for model-id families that
/// cannot be Lightspeed's text-generation route; it is not a capability
/// catalog or a positive compatibility claim.
fn is_openai_selectable_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    ![
        "text-embedding-",
        "text-moderation-",
        "omni-moderation-",
        "dall-e-",
        "gpt-image-",
        "chatgpt-image-",
        "sora-",
        "whisper-",
        "tts-",
        "gpt-realtime",
        "gpt-live-",
        "gpt-transcribe",
        "realtime-",
        "gpt-audio",
        "audio-",
        "gpt-4o-transcribe",
        "gpt-4o-mini-transcribe",
        "gpt-4o-mini-tts",
        "computer-use-",
    ]
    .iter()
    .any(|prefix| model.starts_with(prefix))
        && !model.contains("-search-preview")
        && !model.contains("-search-api")
        && !model.contains("-deep-research")
}

fn anthropic_reasoning_efforts(
    capabilities: Option<&anthropic::ModelCapabilities>,
) -> Option<Vec<String>> {
    let effort = capabilities?.effort.as_ref()?;
    let efforts = [
        ("low", effort.low.as_ref()),
        ("medium", effort.medium.as_ref()),
        ("high", effort.high.as_ref()),
        ("max", effort.max.as_ref()),
        ("xhigh", effort.xhigh.as_ref()),
    ]
    .into_iter()
    .filter_map(|(name, support)| support.filter(|support| support.supported).map(|_| name))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    Some(efforts)
}

fn provider_success(
    provider_id: &str,
    api_kinds: &[&str],
    fetched_at_ms: i64,
    source: ModelProviderCredentialSource,
    credential: ModelProviderCredentialStatus,
) -> ModelProviderDiscoveryView {
    ModelProviderDiscoveryView {
        provider_id: provider_id.to_owned(),
        api_kinds: api_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        fetched_at_ms: Some(fetched_at_ms),
        error: None,
        credential,
        credential_source: source,
    }
}

fn discovery_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn provider_failure(
    provider_id: &str,
    api_kinds: &[&str],
    error: &DiscoveryError,
    source: ModelProviderCredentialSource,
) -> ModelProviderDiscoveryView {
    let (credential, credential_source) = error.credential_status(source);
    ModelProviderDiscoveryView {
        provider_id: provider_id.to_owned(),
        api_kinds: api_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        fetched_at_ms: None,
        error: Some(error.sanitized_message()),
        credential,
        credential_source,
    }
}

enum DiscoveryError {
    ProviderKey(ProviderKeyError),
    Provider(LlmApiError),
    Timeout,
}

impl DiscoveryError {
    /// Typed credential status for the client, derived from the failure and
    /// the credential source that was attempted.
    fn credential_status(
        &self,
        attempted: ModelProviderCredentialSource,
    ) -> (ModelProviderCredentialStatus, ModelProviderCredentialSource) {
        match self {
            // A universe row exists but is disabled/legacy/unusable.
            Self::ProviderKey(ProviderKeyError::NotUsable { .. }) => (
                ModelProviderCredentialStatus::Invalid,
                ModelProviderCredentialSource::Universe,
            ),
            // No universe row and no deployment key: the client had nothing to send.
            Self::Provider(LlmApiError::Configuration(_)) => (
                ModelProviderCredentialStatus::Missing,
                ModelProviderCredentialSource::None,
            ),
            Self::Provider(LlmApiError::HttpStatus(error))
                if error.status == 401 || error.status == 403 =>
            {
                (ModelProviderCredentialStatus::Invalid, attempted)
            }
            _ => (ModelProviderCredentialStatus::Configured, attempted),
        }
    }

    fn sanitized_message(&self) -> String {
        match self {
            Self::ProviderKey(ProviderKeyError::NotUsable { .. }) => {
                "provider credential is not usable".to_owned()
            }
            Self::ProviderKey(ProviderKeyError::Backend { .. }) => {
                "provider credential lookup failed".to_owned()
            }
            Self::Provider(LlmApiError::Configuration(_)) => {
                "provider credential is not configured".to_owned()
            }
            Self::Provider(LlmApiError::Transport(_)) => "provider request failed".to_owned(),
            Self::Provider(LlmApiError::HttpStatus(error)) => {
                format!("provider returned HTTP {}", error.status)
            }
            Self::Provider(LlmApiError::Decode(_)) => {
                "provider returned an invalid model list".to_owned()
            }
            Self::Provider(LlmApiError::Stream(_))
            | Self::Provider(LlmApiError::Unsupported(_)) => {
                "provider model discovery is unavailable".to_owned()
            }
            Self::Timeout => "provider model discovery timed out".to_owned(),
        }
    }
}

pub(super) fn stored_provider_key_resolver(
    store: Arc<PgStore>,
    token_client: Arc<dyn auth::OAuthTokenClient>,
    github_api: Arc<dyn auth::GitHubApiClient>,
) -> Arc<dyn ModelProviderResolver> {
    let grants: Arc<dyn AuthGrantStore> = store.clone();
    let secrets: Arc<dyn SecretStore> = store.clone();
    let clients: Arc<dyn OAuthClientStore> = store.clone();
    let providers: Arc<dyn AuthProviderStore> = store.clone();
    let locks: Arc<dyn GrantRefreshLock> = store.clone();
    let broker: Arc<dyn AuthTokenBroker> = Arc::new(
        RegistryTokenBroker::new(grants.clone(), secrets.clone(), locks)
            .with_oauth_refresh(OAuthRefreshRuntime::new(clients, token_client))
            .with_token_source(
                AuthProviderKind::GitHubApp,
                Arc::new(GitHubAppRuntime::new(
                    providers.clone(),
                    github_api,
                    grants,
                    secrets.clone(),
                )),
            ),
    );
    Arc::new(crate::worker::StoredProviderKeyResolver::new(
        providers, secrets, broker,
    ))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use auth::{
        AuthProviderConfig, AuthProviderId, AuthProviderStatus, CreateAuthProviderRecord,
        InMemoryAuthGrantStore, InMemoryAuthProviderStore, InMemoryGrantLocks, InMemorySecretStore,
        ModelEndpointConfig, ModelEndpointOnlyConfig, RegistryTokenBroker,
    };
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
    };

    use super::*;

    async fn model_list_server() -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = stream.read(&mut buffer).await.expect("read");
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = r#"{"object":"list","data":[{"id":"local-model","object":"model","created":1,"owned_by":"local"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.expect("write");
            String::from_utf8(bytes).expect("request")
        });
        (format!("http://{address}/v1"), task)
    }

    #[test]
    fn anthropic_effort_normalization_keeps_provider_vocabulary() {
        let capabilities = anthropic::ModelCapabilities {
            effort: Some(anthropic::EffortCapability {
                low: Some(anthropic::CapabilitySupport { supported: true }),
                medium: Some(anthropic::CapabilitySupport { supported: false }),
                high: Some(anthropic::CapabilitySupport { supported: true }),
                max: Some(anthropic::CapabilitySupport { supported: true }),
                xhigh: None,
            }),
        };
        assert_eq!(
            anthropic_reasoning_efforts(Some(&capabilities)),
            Some(vec!["low".to_owned(), "high".to_owned(), "max".to_owned()])
        );
    }

    #[test]
    fn provider_errors_do_not_expose_raw_upstream_messages() {
        let error = DiscoveryError::Provider(LlmApiError::Decode(
            llm_clients::DecodeError::with_raw("invalid", "secret upstream body"),
        ));
        assert_eq!(
            error.sanitized_message(),
            "provider returned an invalid model list"
        );
    }

    #[test]
    fn credential_status_distinguishes_missing_invalid_and_configured() {
        use ModelProviderCredentialSource as Source;
        use ModelProviderCredentialStatus as Status;

        let missing = DiscoveryError::Provider(LlmApiError::Configuration(
            llm_clients::ConfigurationError::new("OPENAI_API_KEY must be set"),
        ));
        assert_eq!(
            missing.credential_status(Source::Deployment),
            (Status::Missing, Source::None)
        );

        let unusable = DiscoveryError::ProviderKey(ProviderKeyError::NotUsable {
            provider_id: "openai".to_owned(),
            message: "disabled".to_owned(),
        });
        assert_eq!(
            unusable.credential_status(Source::Deployment),
            (Status::Invalid, Source::Universe)
        );

        let rejected = DiscoveryError::Provider(LlmApiError::HttpStatus(Box::new(
            llm_clients::ProviderHttpError::new(
                "openai:responses",
                reqwest::StatusCode::UNAUTHORIZED,
                "unauthorized",
                Default::default(),
            ),
        )));
        assert_eq!(
            rejected.credential_status(Source::Universe),
            (Status::Invalid, Source::Universe)
        );

        let transport = DiscoveryError::Provider(LlmApiError::Decode(
            llm_clients::DecodeError::with_raw("invalid", "body"),
        ));
        assert_eq!(
            transport.credential_status(Source::Deployment),
            (Status::Configured, Source::Deployment)
        );
    }

    #[test]
    fn selectable_policy_removes_only_clearly_non_generation_openai_families() {
        for model in [
            "text-embedding-3-large",
            "omni-moderation-latest",
            "gpt-image-1",
            "chatgpt-image-latest",
            "sora-2",
            "whisper-1",
            "tts-1",
            "gpt-realtime",
            "gpt-live-transcribe",
            "computer-use-preview",
            "gpt-4o-search-preview",
            "gpt-5-search-api",
            "o3-deep-research",
            "gpt-4o-mini-transcribe",
        ] {
            assert!(!is_openai_selectable_model(model), "{model}");
        }
        for model in ["gpt-5", "gpt-4o-mini", "o4-mini", "custom-fine-tune"] {
            assert!(is_openai_selectable_model(model), "{model}");
        }
    }

    #[test]
    fn openai_models_are_exposed_through_both_registered_api_kinds() {
        let views = openai_model_views(
            openai::Model {
                id: "gpt-5.5".to_owned(),
                created: Some(1_700_000_000),
                object: Some("model".to_owned()),
                owned_by: Some("system".to_owned()),
            },
            42,
        );

        assert_eq!(
            views.clone().map(|view| view.api_kind),
            [
                OPENAI_RESPONSES_API_KIND.to_owned(),
                OPENAI_COMPLETIONS_API_KIND.to_owned(),
            ]
        );
        for view in views {
            assert_eq!(view.created_at_ms, Some(1_700_000_000_000));
            assert_eq!(
                view.capabilities.reasoning_efforts,
                Some(
                    ["none", "low", "medium", "high", "xhigh"]
                        .map(str::to_owned)
                        .to_vec()
                )
            );
        }
    }

    #[test]
    fn openai_reasoning_catalog_covers_current_families_and_snapshots() {
        for model in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert_eq!(
                openai_model_capabilities(model).reasoning_efforts,
                Some(
                    ["none", "low", "medium", "high", "xhigh", "max"]
                        .map(str::to_owned)
                        .to_vec()
                ),
                "{model}"
            );
        }
        assert_eq!(
            openai_model_capabilities("gpt-5.4-mini-2026-03-17").reasoning_efforts,
            Some(
                ["none", "low", "medium", "high", "xhigh"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
        assert_eq!(
            openai_model_capabilities("gpt-5.5-pro-2026-04-23").reasoning_efforts,
            Some(["medium", "high", "xhigh"].map(str::to_owned).to_vec())
        );
        assert_eq!(
            openai_model_capabilities("gpt-5.1-2025-11-13").reasoning_efforts,
            Some(
                ["none", "low", "medium", "high"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
        assert_eq!(
            openai_model_capabilities("gpt-5-2025-08-07").reasoning_efforts,
            Some(
                ["minimal", "low", "medium", "high"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
        assert_eq!(
            openai_model_capabilities("gpt-5.3-chat-latest").reasoning_efforts,
            None
        );
    }

    #[test]
    fn selectable_openai_catalog_omits_stale_dated_models_but_keeps_unknown_dates() {
        let fetched_at_ms = 2_000_000_000_000;
        assert!(is_openai_recent_model(
            Some(fetched_at_ms - OPENAI_SELECTABLE_MAX_AGE_MS),
            fetched_at_ms
        ));
        assert!(!is_openai_recent_model(
            Some(fetched_at_ms - OPENAI_SELECTABLE_MAX_AGE_MS - 1),
            fetched_at_ms
        ));
        assert!(is_openai_recent_model(None, fetched_at_ms));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_endpoint_discovery_uses_the_provider_row_and_declared_api_kinds() {
        let (base_url, server) = model_list_server().await;
        let endpoint = ModelEndpointConfig {
            base_url: base_url.clone(),
            headers: BTreeMap::from([("x-client".to_owned(), "lightspeed".to_owned())]),
            api_kinds: vec![OPENAI_COMPLETIONS_API_KIND.to_owned()],
        };
        let providers = Arc::new(InMemoryAuthProviderStore::new());
        providers
            .create_auth_provider(CreateAuthProviderRecord {
                provider_id: AuthProviderId::new("model:ollama"),
                display_name: Some("Ollama".to_owned()),
                config: AuthProviderConfig::ModelEndpoint(ModelEndpointOnlyConfig {
                    endpoint: endpoint.clone(),
                }),
                credential_secret: None,
                status: AuthProviderStatus::Active,
                created_at_ms: 1,
            })
            .await
            .expect("provider row");
        let secrets = Arc::new(InMemorySecretStore::new());
        let resolver = crate::worker::StoredProviderKeyResolver::new(
            providers.clone(),
            secrets.clone(),
            Arc::new(RegistryTokenBroker::new(
                Arc::new(InMemoryAuthGrantStore::new()),
                secrets,
                Arc::new(InMemoryGrantLocks::new()),
            )),
        );
        let service = ModelDiscoveryService::new(
            Arc::new(
                openai::Client::new(openai::Config::without_api_key()).expect("OpenAI client"),
            ),
            Arc::new(
                anthropic::Client::new(anthropic::Config::without_api_key())
                    .expect("Anthropic client"),
            ),
            Arc::new(resolver),
            providers,
        );

        let (models, statuses) = service.list_custom_providers().await;

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider_id, "ollama");
        assert_eq!(models[0].api_kind, OPENAI_COMPLETIONS_API_KIND);
        assert_eq!(models[0].model, "local-model");
        assert_eq!(statuses.len(), 1);
        assert_eq!(
            statuses[0].credential,
            ModelProviderCredentialStatus::NotRequired
        );
        let request = server.await.expect("server").to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models http/1.1"));
        assert!(request.contains("x-client: lightspeed"));
        assert!(!request.contains("authorization:"));
    }
}
