use forge_llm::{ToolCall, Usage};
use serde::{Deserialize, Serialize};

pub type Timestamp = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserTurn {
    pub content: String,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub reasoning: Option<String>,
    pub usage: Usage,
    pub response_id: Option<String>,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultTurn {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultsTurn {
    pub results: Vec<ToolResultTurn>,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemTurn {
    pub content: String,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SteeringTurn {
    pub content: String,
    pub timestamp: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Turn {
    User(UserTurn),
    Assistant(AssistantTurn),
    ToolResults(ToolResultsTurn),
    System(SystemTurn),
    Steering(SteeringTurn),
}
