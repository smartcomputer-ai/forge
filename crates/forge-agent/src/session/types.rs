use super::Session;
use forge_cxdb_runtime::CxdbTurnId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::{self, Display};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubmitOptions {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub system_prompt_override: Option<String>,
    pub provider_options: Option<Value>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubmitResult {
    pub final_state: SessionState,
    pub assistant_text: String,
    pub tool_call_count: usize,
    pub tool_call_ids: Vec<String>,
    pub tool_error_count: usize,
    pub usage: Option<forge_llm::Usage>,
    pub thread_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub session_id: String,
    pub state: SessionState,
    pub history: Vec<super::Turn>,
    pub steering_queue: Vec<String>,
    pub followup_queue: Vec<String>,
    pub config: super::SessionConfig,
    pub thread_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPersistenceSnapshot {
    pub session_id: String,
    pub context_id: Option<String>,
    pub head_turn_id: Option<CxdbTurnId>,
}

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

pub(super) struct SubAgentRecord {
    pub(super) session: Option<Box<Session>>,
    pub(super) active_task: Option<tokio::task::JoinHandle<SubAgentTaskOutput>>,
    pub(super) result: Option<SubAgentResult>,
}

pub(super) struct SubAgentTaskOutput {
    pub(super) session: Box<Session>,
    pub(super) result: SubAgentResult,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub output: String,
    pub success: bool,
    pub turns_used: usize,
}
