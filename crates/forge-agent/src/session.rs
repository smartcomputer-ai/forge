use crate::{
    AgentError, EventData, EventEmitter, EventKind, ExecutionEnvironment, NoopEventEmitter,
    ProviderProfile, SessionConfig, SessionEvent, Turn,
};
use forge_llm::Client;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Processing,
    AwaitingInput,
    Closed,
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

    pub fn set_state(&mut self, state: SessionState) {
        self.state = state;
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
            return Err(AgentError::SessionClosed);
        }
        self.steering_queue.push_back(message.into());
        Ok(())
    }

    pub fn follow_up(&mut self, message: impl Into<String>) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::SessionClosed);
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
        self.state = SessionState::Closed;
        self.emit(EventKind::SessionEnd, EventData::new())
    }

    pub fn subagents(&self) -> &HashMap<String, SubAgentHandle> {
        &self.subagents
    }

    pub fn emit(&self, kind: EventKind, data: EventData) -> Result<(), AgentError> {
        self.event_emitter.emit(SessionEvent {
            kind,
            timestamp: current_timestamp(),
            session_id: self.id.clone(),
            data,
        })
    }
}

fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BufferedEventEmitter, LocalExecutionEnvironment, ProviderCapabilities,
        StaticProviderProfile, ToolRegistry,
    };
    use forge_llm::Client;
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
        assert!(matches!(err, AgentError::SessionClosed));
    }
}
