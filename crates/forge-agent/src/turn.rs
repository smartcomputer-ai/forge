use forge_llm::{ToolCall, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub content: Value,
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

impl UserTurn {
    pub fn new(content: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            content: content.into(),
            timestamp,
        }
    }
}

impl AssistantTurn {
    pub fn new(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        reasoning: Option<String>,
        usage: Usage,
        response_id: Option<String>,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            content: content.into(),
            tool_calls,
            reasoning,
            usage,
            response_id,
            timestamp,
        }
    }
}

impl ToolResultsTurn {
    pub fn new(results: Vec<ToolResultTurn>, timestamp: Timestamp) -> Self {
        Self { results, timestamp }
    }
}

impl SystemTurn {
    pub fn new(content: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            content: content.into(),
            timestamp,
        }
    }
}

impl SteeringTurn {
    pub fn new(content: impl Into<String>, timestamp: Timestamp) -> Self {
        Self {
            content: content.into(),
            timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_result_turn_preserves_structured_content() {
        let result = ToolResultTurn {
            tool_call_id: "call-1".to_string(),
            content: json!({"stdout":"ok","exit_code":0}),
            is_error: false,
        };

        assert_eq!(result.content["stdout"], "ok");
        assert_eq!(result.content["exit_code"], 0);
    }
}
