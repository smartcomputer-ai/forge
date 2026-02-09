//! Stream accumulator that builds a Response from StreamEvent sequences.

use std::collections::HashMap;

use crate::stream::{StreamEvent, StreamEventType, StreamEventTypeOrString};
use crate::types::{
    ContentPart, FinishReason, Message, Role, ToolCall, ToolCallData, Usage,
};
use crate::Response;

#[derive(Clone, Debug)]
pub struct ResponseSeed {
    pub id: String,
    pub model: String,
    pub provider: String,
}

impl Default for ResponseSeed {
    fn default() -> Self {
        Self {
            id: String::new(),
            model: String::new(),
            provider: String::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct StreamAccumulator {
    seed: ResponseSeed,
    text_order: Vec<String>,
    text_segments: HashMap<String, String>,
    reasoning: String,
    tool_call_order: Vec<String>,
    tool_calls: HashMap<String, ToolCallData>,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
    response: Option<Response>,
}

impl StreamAccumulator {
    pub fn new(seed: ResponseSeed) -> Self {
        Self {
            seed,
            ..Default::default()
        }
    }

    pub fn process(&mut self, event: &StreamEvent) {
        if let Some(response) = &event.response {
            self.response = Some(response.clone());
        }

        match &event.event_type {
            StreamEventTypeOrString::Known(kind) => match kind {
                StreamEventType::TextStart => {
                    let id = event.text_id.clone().unwrap_or_else(|| "default".to_string());
                    if !self.text_segments.contains_key(&id) {
                        self.text_order.push(id.clone());
                        self.text_segments.insert(id, String::new());
                    }
                }
                StreamEventType::TextDelta => {
                    let id = event.text_id.clone().unwrap_or_else(|| "default".to_string());
                    let entry = self.text_segments.entry(id.clone()).or_default();
                    if !self.text_order.contains(&id) {
                        self.text_order.push(id.clone());
                    }
                    if let Some(delta) = &event.delta {
                        entry.push_str(delta);
                    }
                }
                StreamEventType::ReasoningDelta => {
                    if let Some(delta) = &event.reasoning_delta {
                        self.reasoning.push_str(delta);
                    }
                }
                StreamEventType::ToolCallStart
                | StreamEventType::ToolCallDelta
                | StreamEventType::ToolCallEnd => {
                    if let Some(tool_call) = &event.tool_call {
                        self.upsert_tool_call(tool_call);
                    }
                }
                StreamEventType::Finish => {
                    if let Some(reason) = &event.finish_reason {
                        self.finish_reason = Some(reason.clone());
                    }
                    if let Some(usage) = &event.usage {
                        self.usage = Some(usage.clone());
                    }
                }
                _ => {}
            },
            StreamEventTypeOrString::Other(_) => {}
        }
    }

    pub fn response(&self) -> Response {
        if let Some(response) = &self.response {
            return response.clone();
        }

        let mut parts = Vec::new();
        for id in &self.text_order {
            if let Some(text) = self.text_segments.get(id) {
                if !text.is_empty() {
                    parts.push(ContentPart::text(text.clone()));
                }
            }
        }

        if !self.reasoning.is_empty() {
            parts.push(ContentPart::thinking(crate::types::ThinkingData {
                text: self.reasoning.clone(),
                signature: None,
                redacted: false,
            }));
        }

        for id in &self.tool_call_order {
            if let Some(call) = self.tool_calls.get(id) {
                parts.push(ContentPart::tool_call(call.clone()));
            }
        }

        let message = Message {
            role: Role::Assistant,
            content: parts,
            name: None,
            tool_call_id: None,
        };

        Response {
            id: self.seed.id.clone(),
            model: self.seed.model.clone(),
            provider: self.seed.provider.clone(),
            message,
            finish_reason: self.finish_reason.clone().unwrap_or(FinishReason {
                reason: "other".to_string(),
                raw: None,
            }),
            usage: self.usage.clone().unwrap_or(Usage {
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                raw: None,
            }),
            raw: None,
            warnings: Vec::new(),
            rate_limit: None,
        }
    }

    fn upsert_tool_call(&mut self, tool_call: &ToolCall) {
        let data = ToolCallData {
            id: tool_call.id.clone(),
            name: tool_call.name.clone(),
            arguments: if let Some(raw) = &tool_call.raw_arguments {
                serde_json::Value::String(raw.clone())
            } else {
                tool_call.arguments.clone()
            },
            r#type: "function".to_string(),
        };
        let id = data.id.clone();
        if !self.tool_calls.contains_key(&id) {
            self.tool_call_order.push(id.clone());
        }
        self.tool_calls.insert(id, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{StreamEvent, StreamEventTypeOrString};
    use crate::types::{FinishReason, Usage};

    #[test]
    fn accumulates_text_and_finish() {
        let seed = ResponseSeed {
            id: "resp".to_string(),
            model: "model".to_string(),
            provider: "provider".to_string(),
        };
        let mut acc = StreamAccumulator::new(seed);

        acc.process(&StreamEvent {
            event_type: StreamEventTypeOrString::Known(StreamEventType::TextStart),
            delta: None,
            text_id: Some("t1".to_string()),
            reasoning_delta: None,
            tool_call: None,
            finish_reason: None,
            usage: None,
            response: None,
            error: None,
            raw: None,
        });
        acc.process(&StreamEvent {
            event_type: StreamEventTypeOrString::Known(StreamEventType::TextDelta),
            delta: Some("Hello".to_string()),
            text_id: Some("t1".to_string()),
            reasoning_delta: None,
            tool_call: None,
            finish_reason: None,
            usage: None,
            response: None,
            error: None,
            raw: None,
        });
        acc.process(&StreamEvent {
            event_type: StreamEventTypeOrString::Known(StreamEventType::Finish),
            delta: None,
            text_id: None,
            reasoning_delta: None,
            tool_call: None,
            finish_reason: Some(FinishReason {
                reason: "stop".to_string(),
                raw: None,
            }),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                raw: None,
            }),
            response: None,
            error: None,
            raw: None,
        });

        let response = acc.response();
        assert_eq!(response.text(), "Hello");
        assert_eq!(response.finish_reason.reason, "stop");
        assert_eq!(response.usage.total_tokens, 3);
    }
}
