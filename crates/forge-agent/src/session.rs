use crate::{
    AgentError, AssistantTurn, EnvironmentContext, EventData, EventEmitter, EventKind, EventStream,
    ExecutionEnvironment, NoopEventEmitter, ProjectDocument, ProviderProfile, SessionConfig,
    SessionError, SessionEvent, SteeringTurn, ToolDispatchOptions, ToolResultTurn, ToolResultsTurn,
    Turn, UserTurn,
};
use forge_llm::{
    Client, ContentPart, Message, Request, Role, ThinkingData, ToolCallData, ToolChoice,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Display};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Idle,
    Processing,
    AwaitingInput,
    Closed,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Processing => "PROCESSING",
            Self::AwaitingInput => "AWAITING_INPUT",
            Self::Closed => "CLOSED",
        }
    }

    pub fn can_transition_to(&self, next: &SessionState) -> bool {
        if self == next {
            return true;
        }

        if *next == SessionState::Closed {
            return true;
        }

        match self {
            SessionState::Idle => matches!(next, SessionState::Processing),
            SessionState::Processing => matches!(
                next,
                SessionState::Processing | SessionState::AwaitingInput | SessionState::Idle
            ),
            SessionState::AwaitingInput => matches!(next, SessionState::Processing),
            SessionState::Closed => false,
        }
    }
}

impl Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubAgentStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubAgentHandle {
    pub id: String,
    pub status: SubAgentStatus,
}

pub struct Session {
    id: String,
    provider_profile: Arc<dyn ProviderProfile>,
    execution_env: Arc<dyn ExecutionEnvironment>,
    history: Vec<Turn>,
    event_emitter: Arc<dyn EventEmitter>,
    config: SessionConfig,
    state: SessionState,
    llm_client: Arc<Client>,
    steering_queue: VecDeque<String>,
    followup_queue: VecDeque<String>,
    subagents: HashMap<String, SubAgentHandle>,
    abort_requested: bool,
}

impl Session {
    pub fn new(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
    ) -> Result<Self, AgentError> {
        Self::new_with_emitter(
            provider_profile,
            execution_env,
            llm_client,
            config,
            Arc::new(NoopEventEmitter),
        )
    }

    pub fn new_with_emitter(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
    ) -> Result<Self, AgentError> {
        let session = Self {
            id: Uuid::new_v4().to_string(),
            provider_profile,
            execution_env,
            history: Vec::new(),
            event_emitter,
            config,
            state: SessionState::Idle,
            llm_client,
            steering_queue: VecDeque::new(),
            followup_queue: VecDeque::new(),
            subagents: HashMap::new(),
            abort_requested: false,
        };
        session.emit(EventKind::SessionStart, EventData::new())?;
        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn set_state(&mut self, state: SessionState) -> Result<(), AgentError> {
        self.transition_to(state)
    }

    pub fn transition_to(&mut self, next_state: SessionState) -> Result<(), AgentError> {
        if !self.state.can_transition_to(&next_state) {
            return Err(SessionError::InvalidStateTransition {
                from: self.state.to_string(),
                to: next_state.to_string(),
            }
            .into());
        }

        if self.state == next_state {
            return Ok(());
        }

        self.state = next_state;
        if self.state == SessionState::Closed {
            self.emit_session_end()?;
        }
        Ok(())
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub fn provider_profile(&self) -> Arc<dyn ProviderProfile> {
        self.provider_profile.clone()
    }

    pub fn execution_env(&self) -> Arc<dyn ExecutionEnvironment> {
        self.execution_env.clone()
    }

    pub fn llm_client(&self) -> Arc<Client> {
        self.llm_client.clone()
    }

    pub fn history(&self) -> &[Turn] {
        &self.history
    }

    pub fn push_turn(&mut self, turn: Turn) {
        self.history.push(turn);
    }

    pub fn steer(&mut self, message: impl Into<String>) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }
        self.steering_queue.push_back(message.into());
        Ok(())
    }

    pub fn follow_up(&mut self, message: impl Into<String>) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }
        self.followup_queue.push_back(message.into());
        Ok(())
    }

    pub fn set_reasoning_effort(
        &mut self,
        reasoning_effort: Option<String>,
    ) -> Result<(), AgentError> {
        if let Some(value) = reasoning_effort.as_deref() {
            validate_reasoning_effort(value)?;
        }
        self.config.reasoning_effort = reasoning_effort.map(|value| value.to_ascii_lowercase());
        Ok(())
    }

    pub fn reasoning_effort(&self) -> Option<&str> {
        self.config.reasoning_effort.as_deref()
    }

    pub fn pop_steering_message(&mut self) -> Option<String> {
        self.steering_queue.pop_front()
    }

    pub fn pop_followup_message(&mut self) -> Option<String> {
        self.followup_queue.pop_front()
    }

    pub fn request_abort(&mut self) {
        self.abort_requested = true;
    }

    pub async fn process_input(&mut self, user_input: impl Into<String>) -> Result<(), AgentError> {
        self.submit(user_input).await
    }

    pub async fn submit(&mut self, user_input: impl Into<String>) -> Result<(), AgentError> {
        let mut pending_inputs = VecDeque::from([user_input.into()]);

        while let Some(next_input) = pending_inputs.pop_front() {
            let completed_naturally = self.submit_single(next_input).await?;
            if completed_naturally {
                while let Some(follow_up) = self.pop_followup_message() {
                    pending_inputs.push_back(follow_up);
                }
            }
        }

        Ok(())
    }

    async fn submit_single(&mut self, user_input: String) -> Result<bool, AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }

        if self.abort_requested {
            self.transition_to(SessionState::Closed)?;
            return Ok(false);
        }

        self.transition_to(SessionState::Processing)?;
        self.push_turn(Turn::User(UserTurn::new(
            user_input.clone(),
            current_timestamp(),
        )));
        self.emit(
            EventKind::UserInput,
            EventData::from_serializable(serde_json::json!({ "content": user_input }))?,
        )?;
        self.drain_steering_queue()?;

        let mut round_count = 0usize;
        let mut completed_naturally = false;
        loop {
            if self.abort_requested {
                self.transition_to(SessionState::Closed)?;
                return Ok(false);
            }

            if round_count >= self.config.max_tool_rounds_per_input {
                self.event_emitter
                    .emit(SessionEvent::turn_limit_round(self.id.clone(), round_count))?;
                break;
            }

            if self.config.max_turns > 0 && self.history.len() >= self.config.max_turns {
                self.emit(
                    EventKind::TurnLimit,
                    EventData::from_serializable(serde_json::json!({
                        "total_turns": self.history.len()
                    }))?,
                )?;
                break;
            }

            let request = self.build_request();
            self.emit(EventKind::AssistantTextStart, EventData::new())?;
            let response = match self.llm_client.complete(request).await {
                Ok(response) => response,
                Err(error) => {
                    self.event_emitter
                        .emit(SessionEvent::error(self.id.clone(), error.to_string()))?;
                    self.transition_to(SessionState::Closed)?;
                    return Err(error.into());
                }
            };

            let text = response.text();
            let tool_calls = response.tool_calls();
            let reasoning = response.reasoning();
            self.push_turn(Turn::Assistant(AssistantTurn::new(
                text.clone(),
                tool_calls.clone(),
                reasoning.clone(),
                response.usage.clone(),
                Some(response.id),
                current_timestamp(),
            )));
            self.event_emitter.emit(SessionEvent::assistant_text_end(
                self.id.clone(),
                text,
                reasoning,
            ))?;

            if tool_calls.is_empty() {
                completed_naturally = true;
                break;
            }

            round_count += 1;
            let results = self
                .provider_profile
                .tool_registry()
                .dispatch(
                    tool_calls,
                    self.execution_env.clone(),
                    &self.config,
                    self.event_emitter.clone(),
                    ToolDispatchOptions {
                        session_id: self.id.clone(),
                        supports_parallel_tool_calls: self
                            .provider_profile
                            .capabilities()
                            .supports_parallel_tool_calls,
                    },
                )
                .await?;
            let result_turns = results
                .into_iter()
                .map(|result| ToolResultTurn {
                    tool_call_id: result.tool_call_id,
                    content: result.content,
                    is_error: result.is_error,
                })
                .collect();
            self.push_turn(Turn::ToolResults(ToolResultsTurn::new(
                result_turns,
                current_timestamp(),
            )));
            self.drain_steering_queue()?;
            self.inject_loop_detection_warning_if_needed()?;
        }

        if self.state != SessionState::Closed {
            self.transition_to(SessionState::Idle)?;
        }
        Ok(completed_naturally)
    }

    pub fn close(&mut self) -> Result<(), AgentError> {
        self.transition_to(SessionState::Closed)
    }

    pub fn subagents(&self) -> &HashMap<String, SubAgentHandle> {
        &self.subagents
    }

    pub fn subscribe_events(&self) -> EventStream {
        self.event_emitter.subscribe()
    }

    pub fn emit(&self, kind: EventKind, data: EventData) -> Result<(), AgentError> {
        self.event_emitter
            .emit(SessionEvent::new(kind, self.id.clone(), data))
    }

    fn emit_session_end(&self) -> Result<(), AgentError> {
        self.event_emitter.emit(SessionEvent::session_end(
            self.id.clone(),
            self.state.to_string(),
        ))
    }

    fn drain_steering_queue(&mut self) -> Result<(), AgentError> {
        while let Some(content) = self.pop_steering_message() {
            self.push_turn(Turn::Steering(SteeringTurn::new(
                content.clone(),
                current_timestamp(),
            )));
            self.event_emitter
                .emit(SessionEvent::steering_injected(self.id.clone(), content))?;
        }
        Ok(())
    }

    fn inject_loop_detection_warning_if_needed(&mut self) -> Result<(), AgentError> {
        if !self.config.enable_loop_detection {
            return Ok(());
        }

        if !detect_loop(&self.history, self.config.loop_detection_window) {
            return Ok(());
        }

        let warning = format!(
            "Loop detected: the last {} tool calls follow a repeating pattern. Try a different approach.",
            self.config.loop_detection_window
        );
        if matches!(
            self.history.last(),
            Some(Turn::Steering(turn)) if turn.content == warning
        ) {
            return Ok(());
        }

        self.push_turn(Turn::Steering(SteeringTurn::new(
            warning.clone(),
            current_timestamp(),
        )));
        self.event_emitter
            .emit(SessionEvent::loop_detection(self.id.clone(), warning))?;
        Ok(())
    }

    fn build_request(&self) -> Request {
        let environment = self.build_environment_context();
        let project_docs: Vec<ProjectDocument> = Vec::new();
        let system_prompt = self
            .provider_profile
            .build_system_prompt(&environment, &project_docs);

        let mut messages = vec![Message::system(system_prompt)];
        messages.extend(convert_history_to_messages(&self.history));

        let tools = self.provider_profile.tools();
        let tools = if tools.is_empty() { None } else { Some(tools) };
        let tool_choice = tools.as_ref().map(|_| ToolChoice {
            mode: "auto".to_string(),
            tool_name: None,
        });

        Request {
            model: self.provider_profile.model().to_string(),
            messages,
            provider: Some(self.provider_profile.id().to_string()),
            tools,
            tool_choice,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort: self.config.reasoning_effort.clone(),
            metadata: None,
            provider_options: self.provider_profile.provider_options(),
        }
    }

    fn build_environment_context(&self) -> EnvironmentContext {
        let working_directory = self.execution_env.working_directory();
        let is_git_repository = working_directory.join(".git").exists();

        EnvironmentContext {
            working_directory: working_directory.to_string_lossy().to_string(),
            platform: self.execution_env.platform().to_string(),
            os_version: self.execution_env.os_version().to_string(),
            is_git_repository,
            git_branch: None,
            date_yyyy_mm_dd: current_date_yyyy_mm_dd(),
            model: self.provider_profile.model().to_string(),
            knowledge_cutoff: None,
        }
    }
}

fn convert_history_to_messages(history: &[Turn]) -> Vec<Message> {
    let mut messages = Vec::new();

    for turn in history {
        match turn {
            Turn::User(turn) => messages.push(Message::user(turn.content.clone())),
            Turn::Assistant(turn) => {
                let mut content = Vec::new();
                if !turn.content.is_empty() {
                    content.push(ContentPart::text(turn.content.clone()));
                }

                if let Some(reasoning) = &turn.reasoning {
                    if !reasoning.is_empty() {
                        content.push(ContentPart::thinking(ThinkingData {
                            text: reasoning.clone(),
                            signature: None,
                            redacted: false,
                        }));
                    }
                }

                for tool_call in &turn.tool_calls {
                    content.push(ContentPart::tool_call(ToolCallData {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                        r#type: "function".to_string(),
                    }));
                }

                if content.is_empty() {
                    content.push(ContentPart::text(String::new()));
                }

                messages.push(Message {
                    role: Role::Assistant,
                    content,
                    name: None,
                    tool_call_id: None,
                });
            }
            Turn::ToolResults(turn) => {
                for result in &turn.results {
                    messages.push(Message::tool_result(
                        result.tool_call_id.clone(),
                        result.content.clone(),
                        result.is_error,
                    ));
                }
            }
            Turn::System(turn) => messages.push(Message::system(turn.content.clone())),
            Turn::Steering(turn) => messages.push(Message::user(turn.content.clone())),
        }
    }

    messages
}

fn current_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

fn current_date_yyyy_mm_dd() -> String {
    #[cfg(windows)]
    let command = ("cmd", vec!["/C", "echo %date%"]);
    #[cfg(not(windows))]
    let command = ("date", vec!["+%Y-%m-%d"]);

    let output = std::process::Command::new(command.0)
        .args(command.1)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    "1970-01-01".to_string()
}

fn validate_reasoning_effort(value: &str) -> Result<(), AgentError> {
    let normalized = value.to_ascii_lowercase();
    match normalized.as_str() {
        "low" | "medium" | "high" => Ok(()),
        _ => Err(SessionError::InvalidConfiguration(format!(
            "reasoning_effort must be one of: low, medium, high (received '{}')",
            value
        ))
        .into()),
    }
}

fn detect_loop(history: &[Turn], window_size: usize) -> bool {
    if window_size == 0 {
        return false;
    }

    let signatures: Vec<u64> = history
        .iter()
        .filter_map(|turn| {
            if let Turn::Assistant(turn) = turn {
                Some(
                    turn.tool_calls
                        .iter()
                        .map(tool_call_signature)
                        .collect::<Vec<u64>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect();

    if signatures.len() < window_size {
        return false;
    }

    let recent = &signatures[signatures.len() - window_size..];
    for pattern_len in 1..=3 {
        if window_size % pattern_len != 0 {
            continue;
        }

        let pattern = &recent[0..pattern_len];
        let mut all_match = true;
        for chunk in recent.chunks(pattern_len).skip(1) {
            if chunk != pattern {
                all_match = false;
                break;
            }
        }
        if all_match {
            return true;
        }
    }

    false
}

fn tool_call_signature(tool_call: &forge_llm::ToolCall) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_call.name.hash(&mut hasher);
    if let Ok(serialized) = serde_json::to_string(&tool_call.arguments) {
        serialized.hash(&mut hasher);
    } else {
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        raw_arguments.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BufferedEventEmitter, LocalExecutionEnvironment, ProviderCapabilities, RegisteredTool,
        StaticProviderProfile, ToolExecutor, ToolRegistry,
    };
    use async_trait::async_trait;
    use forge_llm::{
        Client, ConfigurationError, ContentPart, FinishReason, Message, ProviderAdapter, Request,
        Response, SDKError, StreamEventStream, ToolCallData, Usage,
    };
    use futures::{StreamExt, executor::block_on};
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct SequenceAdapter {
        responses: Arc<Mutex<VecDeque<Response>>>,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    #[async_trait]
    impl ProviderAdapter for SequenceAdapter {
        fn name(&self) -> &str {
            "test"
        }

        async fn complete(&self, request: Request) -> Result<Response, SDKError> {
            self.requests.lock().expect("requests mutex").push(request);
            self.responses
                .lock()
                .expect("responses mutex")
                .pop_front()
                .ok_or_else(|| {
                    SDKError::Configuration(ConfigurationError::new("no response queued"))
                })
        }

        async fn stream(&self, _request: Request) -> Result<StreamEventStream, SDKError> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn test_usage() -> Usage {
        Usage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            raw: None,
        }
    }

    fn text_response(id: &str, text: &str) -> Response {
        Response {
            id: id.to_string(),
            model: "gpt-5.2-codex".to_string(),
            provider: "test".to_string(),
            message: Message::assistant(text),
            finish_reason: FinishReason {
                reason: "stop".to_string(),
                raw: None,
            },
            usage: test_usage(),
            raw: None,
            warnings: Vec::new(),
            rate_limit: None,
        }
    }

    fn tool_call_response(id: &str, call_id: &str, tool_name: &str, args: Value) -> Response {
        Response {
            id: id.to_string(),
            model: "gpt-5.2-codex".to_string(),
            provider: "test".to_string(),
            message: Message {
                role: forge_llm::Role::Assistant,
                content: vec![ContentPart::tool_call(ToolCallData {
                    id: call_id.to_string(),
                    name: tool_name.to_string(),
                    arguments: args,
                    r#type: "function".to_string(),
                })],
                name: None,
                tool_call_id: None,
            },
            finish_reason: FinishReason {
                reason: "tool_calls".to_string(),
                raw: None,
            },
            usage: test_usage(),
            raw: None,
            warnings: Vec::new(),
            rate_limit: None,
        }
    }

    fn build_test_client(responses: Vec<Response>) -> (Arc<Client>, Arc<Mutex<Vec<Request>>>) {
        let adapter = Arc::new(SequenceAdapter {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        });

        let requests = adapter.requests.clone();
        let mut client = Client::default();
        client
            .register_provider(adapter)
            .expect("provider should register");
        (Arc::new(client), requests)
    }

    fn tool_registry_with_echo() -> Arc<ToolRegistry> {
        tool_registry_with_named_echoes(&["echo_tool"])
    }

    fn tool_registry_with_named_echoes(names: &[&str]) -> Arc<ToolRegistry> {
        let mut tool_registry = ToolRegistry::default();
        for name in names {
            let executor: ToolExecutor = Arc::new(|args, _env| {
                Box::pin(async move {
                    let output = args
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or("missing")
                        .to_string();
                    Ok(output)
                })
            });
            tool_registry.register(RegisteredTool {
                definition: forge_llm::ToolDefinition {
                    name: (*name).to_string(),
                    description: "echo".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "required": ["value"],
                        "properties": {
                            "value": { "type": "string" }
                        }
                    }),
                },
                executor,
            });
        }
        Arc::new(tool_registry)
    }

    #[test]
    fn session_new_emits_session_start() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("session should initialize");

        assert!(!session.id().is_empty());
        let events = emitter.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::SessionStart);
    }

    #[test]
    fn session_rejects_steer_when_closed() {
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session.close().expect("close should succeed");

        let err = session.steer("halt").expect_err("steer should fail");
        assert!(matches!(err, AgentError::Session(SessionError::Closed)));
    }

    #[test]
    fn session_state_enforces_spec_transitions() {
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .transition_to(SessionState::Processing)
            .expect("idle -> processing should work");
        session
            .transition_to(SessionState::AwaitingInput)
            .expect("processing -> awaiting_input should work");
        session
            .transition_to(SessionState::Processing)
            .expect("awaiting_input -> processing should work");
        session
            .transition_to(SessionState::Idle)
            .expect("processing -> idle should work");

        let err = session
            .transition_to(SessionState::AwaitingInput)
            .expect_err("idle -> awaiting_input should fail");
        assert!(matches!(
            err,
            AgentError::Session(SessionError::InvalidStateTransition { .. })
        ));
    }

    #[test]
    fn closing_session_emits_session_end_once_with_final_state() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("session should initialize");

        session.close().expect("close should succeed");
        session.close().expect("second close should be a no-op");

        let events = emitter.snapshot();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, EventKind::SessionStart);
        assert_eq!(events[1].kind, EventKind::SessionEnd);
        assert_eq!(events[1].data.get_str("final_state"), Some("CLOSED"));
    }

    #[test]
    fn session_exposes_async_event_subscription() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let session =
            Session::new_with_emitter(profile, env, client, SessionConfig::default(), emitter)
                .expect("session should initialize");

        let mut stream = session.subscribe_events();
        session
            .emit(
                EventKind::UserInput,
                EventData::from_serializable(serde_json::json!({ "content": "hi" }))
                    .expect("valid object payload"),
            )
            .expect("emit should succeed");

        let first = block_on(stream.next()).expect("session start should arrive");
        assert_eq!(first.kind, EventKind::SessionStart);
        let second = block_on(stream.next()).expect("user input should arrive");
        assert_eq!(second.kind, EventKind::UserInput);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_natural_completion_without_tool_calls_returns_to_idle() {
        let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .submit("hello")
            .await
            .expect("submit should succeed");

        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 2);
        assert!(matches!(session.history()[0], Turn::User(_)));
        assert!(matches!(session.history()[1], Turn::Assistant(_)));
        assert_eq!(requests.lock().expect("requests mutex").len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_enforces_per_input_round_limit_and_emits_turn_limit_event() {
        let (client, requests) = build_test_client(vec![
            tool_call_response(
                "resp-1",
                "call-1",
                "echo_tool",
                serde_json::json!({ "value": "first" }),
            ),
            text_response("resp-2", "should_not_be_called"),
        ]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: tool_registry_with_echo(),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut config = SessionConfig::default();
        config.max_tool_rounds_per_input = 1;
        let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
            .expect("new session");

        session
            .submit("run tool")
            .await
            .expect("submit should succeed");

        let events = emitter.snapshot();
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::TurnLimit)
        );
        assert_eq!(requests.lock().expect("requests mutex").len(), 1);
        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 3);
        assert!(matches!(session.history()[2], Turn::ToolResults(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_multiple_times_keeps_history_consistent() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session.submit("one").await.expect("first submit");
        session.submit("two").await.expect("second submit");

        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 4);
        assert!(matches!(session.history()[0], Turn::User(_)));
        assert!(matches!(session.history()[1], Turn::Assistant(_)));
        assert!(matches!(session.history()[2], Turn::User(_)));
        assert!(matches!(session.history()[3], Turn::Assistant(_)));
        assert_eq!(requests.lock().expect("requests mutex").len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn steering_messages_are_injected_into_history_and_next_request() {
        let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session
            .steer("Use concise output")
            .expect("steer should queue");

        session
            .submit("hello")
            .await
            .expect("submit should succeed");

        assert!(matches!(session.history()[1], Turn::Steering(_)));
        let requests = requests.lock().expect("requests mutex");
        let first_request = &requests[0];
        assert!(first_request
            .messages
            .iter()
            .any(|message| message.role == Role::User && message.text() == "Use concise output"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn follow_up_queue_triggers_new_processing_cycle_after_completion() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session
            .follow_up("second input")
            .expect("follow-up should queue");

        session
            .submit("first input")
            .await
            .expect("submit should succeed");

        assert_eq!(session.history().len(), 4);
        assert!(matches!(&session.history()[0], Turn::User(turn) if turn.content == "first input"));
        assert!(
            matches!(&session.history()[2], Turn::User(turn) if turn.content == "second input")
        );
        assert_eq!(requests.lock().expect("requests mutex").len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loop_detection_injects_warning_steering_turn_and_event() {
        let (client, requests) = build_test_client(vec![
            tool_call_response(
                "resp-1",
                "call-1",
                "tool_a",
                serde_json::json!({ "value": "a" }),
            ),
            tool_call_response(
                "resp-2",
                "call-2",
                "tool_b",
                serde_json::json!({ "value": "b" }),
            ),
            tool_call_response(
                "resp-3",
                "call-3",
                "tool_a",
                serde_json::json!({ "value": "a" }),
            ),
            tool_call_response(
                "resp-4",
                "call-4",
                "tool_b",
                serde_json::json!({ "value": "b" }),
            ),
            text_response("resp-5", "done"),
        ]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: tool_registry_with_named_echoes(&["tool_a", "tool_b"]),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut config = SessionConfig::default();
        config.loop_detection_window = 4;
        let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
            .expect("new session");

        session
            .submit("start")
            .await
            .expect("submit should succeed");

        assert!(session.history().iter().any(|turn| matches!(
            turn,
            Turn::Steering(turn) if turn.content.contains("Loop detected")
        )));
        assert!(
            emitter
                .snapshot()
                .iter()
                .any(|event| event.kind == EventKind::LoopDetection)
        );

        let requests = requests.lock().expect("requests mutex");
        assert!(requests[4].messages.iter().any(|message| {
            message.role == Role::User && message.text().contains("Loop detected")
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reasoning_effort_updates_apply_to_next_llm_call() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .set_reasoning_effort(Some("low".to_string()))
            .expect("low should be valid");
        session.submit("one").await.expect("first submit");
        session
            .set_reasoning_effort(Some("high".to_string()))
            .expect("high should be valid");
        session.submit("two").await.expect("second submit");

        let requests = requests.lock().expect("requests mutex");
        assert_eq!(requests[0].reasoning_effort.as_deref(), Some("low"));
        assert_eq!(requests[1].reasoning_effort.as_deref(), Some("high"));

        let err = session
            .set_reasoning_effort(Some("ultra".to_string()))
            .expect_err("invalid value should be rejected");
        assert!(matches!(
            err,
            AgentError::Session(SessionError::InvalidConfiguration(_))
        ));
    }
}
