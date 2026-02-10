use async_trait::async_trait;
use forge_agent::{
    AgentError, BufferedEventEmitter, EventKind, LocalExecutionEnvironment, ProviderProfile,
    Session, SessionConfig, SessionEvent, SubmitOptions, Turn,
};
use forge_llm::{
    AnthropicAdapter, AnthropicAdapterConfig, Client, OpenAIAdapter, OpenAIAdapterConfig,
    ProviderAdapter, Request, Response, SDKError, StreamEventStream,
};
use std::env;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::{TempDir, tempdir};
use tokio::time::{Duration, sleep, timeout};

pub const LIVE_RETRIES: usize = 3;
pub const LIVE_SUBMIT_TIMEOUT_SECS: u64 = 180;

#[derive(Clone)]
struct RecordingAdapter {
    name: String,
    inner: Arc<dyn ProviderAdapter>,
    requests: Arc<Mutex<Vec<Request>>>,
}

#[async_trait]
impl ProviderAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, request: Request) -> Result<Response, SDKError> {
        self.requests
            .lock()
            .expect("requests mutex")
            .push(request.clone());
        self.inner.complete(request).await
    }

    async fn stream(&self, request: Request) -> Result<StreamEventStream, SDKError> {
        self.requests
            .lock()
            .expect("requests mutex")
            .push(request.clone());
        self.inner.stream(request).await
    }
}

pub fn live_tests_enabled(flag_name: &str) -> bool {
    match env::var(flag_name) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "1" || normalized == "true" || normalized == "yes"
        }
        Err(_) => false,
    }
}

pub fn openai_live_model() -> String {
    env_or_dotenv_var("OPENAI_LIVE_MODEL").unwrap_or_else(|| "gpt-5-mini".to_string())
}

pub fn anthropic_live_model() -> String {
    env_or_dotenv_var("ANTHROPIC_LIVE_MODEL").unwrap_or_else(|| "claude-sonnet-4-5".to_string())
}

pub fn build_openai_live_client() -> Option<(Arc<Client>, Arc<Mutex<Vec<Request>>>)> {
    let api_key = env_or_dotenv_var("OPENAI_API_KEY")?;
    let mut config = OpenAIAdapterConfig::new(api_key);
    if let Some(base_url) = env_or_dotenv_var("OPENAI_BASE_URL") {
        config.base_url = base_url;
    }
    if let Some(org_id) = env_or_dotenv_var("OPENAI_ORG_ID") {
        config.org_id = Some(org_id);
    }
    if let Some(project_id) = env_or_dotenv_var("OPENAI_PROJECT_ID") {
        config.project_id = Some(project_id);
    }
    let adapter = OpenAIAdapter::new(config).ok()?;
    client_with_recording_adapter("openai", Arc::new(adapter))
}

pub fn build_anthropic_live_client() -> Option<(Arc<Client>, Arc<Mutex<Vec<Request>>>)> {
    let api_key = env_or_dotenv_var("ANTHROPIC_API_KEY")?;
    let mut config = AnthropicAdapterConfig::new(api_key);
    if let Some(base_url) = env_or_dotenv_var("ANTHROPIC_BASE_URL") {
        config.base_url = base_url;
    }
    let adapter = AnthropicAdapter::new(config).ok()?;
    client_with_recording_adapter("anthropic", Arc::new(adapter))
}

fn client_with_recording_adapter(
    provider_name: &str,
    inner: Arc<dyn ProviderAdapter>,
) -> Option<(Arc<Client>, Arc<Mutex<Vec<Request>>>)> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let wrapper = Arc::new(RecordingAdapter {
        name: provider_name.to_string(),
        inner,
        requests: requests.clone(),
    });

    let mut client = Client::default();
    if client.register_provider(wrapper).is_err() {
        return None;
    }

    Some((Arc::new(client), requests))
}

pub fn bootstrap_live_session(
    provider_profile: Arc<dyn ProviderProfile>,
    llm_client: Arc<Client>,
    config: SessionConfig,
) -> Result<
    (
        TempDir,
        Arc<LocalExecutionEnvironment>,
        Arc<BufferedEventEmitter>,
        Session,
    ),
    AgentError,
> {
    let workspace = tempdir().map_err(|error| {
        AgentError::ExecutionEnvironment(format!("failed to create temp workspace: {error}"))
    })?;
    let env = Arc::new(LocalExecutionEnvironment::new(workspace.path()));
    let emitter = Arc::new(BufferedEventEmitter::default());
    let session = Session::new_with_emitter(
        provider_profile,
        env.clone(),
        llm_client,
        config,
        emitter.clone(),
    )?;
    Ok((workspace, env, emitter, session))
}

pub async fn submit_with_timeout(
    session: &mut Session,
    user_input: &str,
) -> Result<(), AgentError> {
    match timeout(
        Duration::from_secs(LIVE_SUBMIT_TIMEOUT_SECS),
        session.submit(user_input),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(AgentError::ExecutionEnvironment(format!(
            "live submit timed out after {}s",
            LIVE_SUBMIT_TIMEOUT_SECS
        ))),
    }
}

pub async fn submit_with_options_timeout(
    session: &mut Session,
    user_input: &str,
    options: SubmitOptions,
) -> Result<(), AgentError> {
    match timeout(
        Duration::from_secs(LIVE_SUBMIT_TIMEOUT_SECS),
        session.submit_with_options(user_input, options),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(AgentError::ExecutionEnvironment(format!(
            "live submit timed out after {}s",
            LIVE_SUBMIT_TIMEOUT_SECS
        ))),
    }
}

pub async fn run_with_retries<T, F, Fut>(mut operation: F) -> Result<T, AgentError>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, AgentError>>,
{
    let mut last_error: Option<AgentError> = None;
    for attempt in 0..LIVE_RETRIES {
        match operation(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) if is_retryable_agent_error(&error) && attempt + 1 < LIVE_RETRIES => {
                last_error = Some(error);
                sleep(Duration::from_millis(250 * (attempt as u64 + 1))).await;
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        AgentError::ExecutionEnvironment("live operation exhausted retries".to_string())
    }))
}

fn is_retryable_agent_error(error: &AgentError) -> bool {
    matches!(error, AgentError::Llm(inner) if inner.retryable())
}

pub fn find_tool_result_with_substring(
    history: &[Turn],
    needle: &str,
) -> Option<(String, String, bool)> {
    for turn in history {
        let Turn::ToolResults(tool_results) = turn else {
            continue;
        };
        for result in &tool_results.results {
            let text = result
                .content
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| result.content.to_string());
            if text.contains(needle) {
                return Some((result.tool_call_id.clone(), text, result.is_error));
            }
        }
    }
    None
}

pub fn collect_tool_results(history: &[Turn]) -> Vec<(String, String, bool)> {
    let mut collected = Vec::new();
    for turn in history {
        let Turn::ToolResults(tool_results) = turn else {
            continue;
        };
        for result in &tool_results.results {
            let text = result
                .content
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| result.content.to_string());
            collected.push((result.tool_call_id.clone(), text, result.is_error));
        }
    }
    collected
}

pub fn find_tool_call_end_output(events: &[SessionEvent], call_id: &str) -> Option<String> {
    events.iter().find_map(|event| {
        if event.kind != EventKind::ToolCallEnd {
            return None;
        }
        if event.data.get_str("call_id") != Some(call_id) {
            return None;
        }
        event.data.get_str("output").map(ToOwned::to_owned)
    })
}

fn dotenv_candidates() -> Vec<PathBuf> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest_dir.join("../../.env"),
        manifest_dir.join(".env"),
        PathBuf::from(".env"),
    ]
}

fn parse_dotenv_value(contents: &str, key: &str) -> Option<String> {
    for raw_line in contents.lines() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(stripped) = line.strip_prefix("export ") {
            line = stripped.trim();
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let value = value.trim();
        let unquoted = if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        if !unquoted.is_empty() {
            return Some(unquoted.to_string());
        }
    }
    None
}

fn env_or_dotenv_var(key: &str) -> Option<String> {
    if let Ok(value) = env::var(key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    for path in dotenv_candidates() {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(value) = parse_dotenv_value(&contents, key) {
            return Some(value);
        }
    }

    None
}
