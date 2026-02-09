use crate::{
    AgentError, EventData, EventEmitter, EventKind, EventStream, ExecutionEnvironment,
    NoopEventEmitter, ProviderProfile, SessionConfig, SessionError, SessionEvent, Turn,
};
use forge_llm::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Display};
use std::sync::Arc;
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

    pub fn pop_steering_message(&mut self) -> Option<String> {
        self.steering_queue.pop_front()
    }

    pub fn pop_followup_message(&mut self) -> Option<String> {
        self.followup_queue.pop_front()
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BufferedEventEmitter, LocalExecutionEnvironment, ProviderCapabilities,
        StaticProviderProfile, ToolRegistry,
    };
    use forge_llm::Client;
    use futures::{StreamExt, executor::block_on};
    use std::path::PathBuf;

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
}
