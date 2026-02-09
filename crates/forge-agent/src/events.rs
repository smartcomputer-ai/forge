use crate::{AgentError, SessionError};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type EventStream = UnboundedReceiver<SessionEvent>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventData {
    inner: HashMap<String, Value>,
}

impl EventData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_value(&mut self, key: impl Into<String>, value: Value) {
        self.inner.insert(key.into(), value);
    }

    pub fn insert_string(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.insert_value(key, Value::String(value.into()));
    }

    pub fn insert_bool(&mut self, key: impl Into<String>, value: bool) {
        self.insert_value(key, Value::Bool(value));
    }

    pub fn insert_u64(&mut self, key: impl Into<String>, value: u64) {
        self.insert_value(key, Value::from(value));
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.inner.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub fn from_serializable<T: Serialize>(value: T) -> Result<Self, AgentError> {
        let json = serde_json::to_value(value)
            .map_err(|err| SessionError::EventSerialization(err.to_string()))?;
        match json {
            Value::Object(map) => Ok(Self {
                inner: map.into_iter().collect(),
            }),
            _ => Err(SessionError::EventSerialization(
                "payload must serialize to a JSON object".to_string(),
            )
            .into()),
        }
    }
}

impl From<HashMap<String, Value>> for EventData {
    fn from(inner: HashMap<String, Value>) -> Self {
        Self { inner }
    }
}

impl From<EventData> for HashMap<String, Value> {
    fn from(value: EventData) -> Self {
        value.inner
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    UserInput,
    AssistantTextStart,
    AssistantTextDelta,
    AssistantTextEnd,
    ToolCallStart,
    ToolCallOutputDelta,
    ToolCallEnd,
    SteeringInjected,
    TurnLimit,
    LoopDetection,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub kind: EventKind,
    pub timestamp: String,
    pub session_id: String,
    pub data: EventData,
}

impl SessionEvent {
    pub fn new(kind: EventKind, session_id: impl Into<String>, data: EventData) -> Self {
        Self {
            kind,
            timestamp: current_timestamp(),
            session_id: session_id.into(),
            data,
        }
    }

    pub fn with_timestamp(
        kind: EventKind,
        timestamp: impl Into<String>,
        session_id: impl Into<String>,
        data: EventData,
    ) -> Self {
        Self {
            kind,
            timestamp: timestamp.into(),
            session_id: session_id.into(),
            data,
        }
    }

    pub fn session_start(session_id: impl Into<String>) -> Self {
        Self::new(EventKind::SessionStart, session_id, EventData::new())
    }

    pub fn session_end(session_id: impl Into<String>, final_state: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("final_state", final_state);
        Self::new(EventKind::SessionEnd, session_id, data)
    }

    pub fn user_input(session_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("content", content);
        Self::new(EventKind::UserInput, session_id, data)
    }

    pub fn assistant_text_end(
        session_id: impl Into<String>,
        text: impl Into<String>,
        reasoning: Option<impl Into<String>>,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string("text", text);
        if let Some(reasoning) = reasoning {
            data.insert_string("reasoning", reasoning);
        }
        Self::new(EventKind::AssistantTextEnd, session_id, data)
    }

    pub fn assistant_text_delta(session_id: impl Into<String>, delta: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("delta", delta);
        Self::new(EventKind::AssistantTextDelta, session_id, data)
    }

    pub fn tool_call_start(
        session_id: impl Into<String>,
        tool_name: impl Into<String>,
        call_id: impl Into<String>,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string("tool_name", tool_name);
        data.insert_string("call_id", call_id);
        Self::new(EventKind::ToolCallStart, session_id, data)
    }

    pub fn tool_call_end_output(
        session_id: impl Into<String>,
        call_id: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string("call_id", call_id);
        data.insert_string("output", output);
        Self::new(EventKind::ToolCallEnd, session_id, data)
    }

    pub fn tool_call_output_delta(
        session_id: impl Into<String>,
        call_id: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string("call_id", call_id);
        data.insert_string("delta", delta);
        Self::new(EventKind::ToolCallOutputDelta, session_id, data)
    }

    pub fn tool_call_end_error(
        session_id: impl Into<String>,
        call_id: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string("call_id", call_id);
        data.insert_string("error", error);
        Self::new(EventKind::ToolCallEnd, session_id, data)
    }

    pub fn steering_injected(session_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("content", content);
        Self::new(EventKind::SteeringInjected, session_id, data)
    }

    pub fn turn_limit_round(session_id: impl Into<String>, round: usize) -> Self {
        let mut data = EventData::new();
        data.insert_u64("round", round as u64);
        Self::new(EventKind::TurnLimit, session_id, data)
    }

    pub fn loop_detection(session_id: impl Into<String>, message: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("message", message);
        Self::new(EventKind::LoopDetection, session_id, data)
    }

    pub fn error(session_id: impl Into<String>, message: impl Into<String>) -> Self {
        let mut data = EventData::new();
        data.insert_string("message", message);
        Self::new(EventKind::Error, session_id, data)
    }

    pub fn context_usage_warning(
        session_id: impl Into<String>,
        approx_tokens: usize,
        context_window_size: usize,
        usage_percent: usize,
    ) -> Self {
        let mut data = EventData::new();
        data.insert_string(
            "message",
            format!("Context usage at ~{}% of context window", usage_percent),
        );
        data.insert_string("severity", "warning");
        data.insert_string("category", "context_usage");
        data.insert_u64("approx_tokens", approx_tokens as u64);
        data.insert_u64("context_window_size", context_window_size as u64);
        data.insert_u64("usage_percent", usage_percent as u64);
        Self::new(EventKind::Warning, session_id, data)
    }
}

pub trait EventEmitter: Send + Sync {
    fn emit(&self, event: SessionEvent) -> Result<(), AgentError>;
    fn subscribe(&self) -> EventStream;
}

#[derive(Default)]
pub struct NoopEventEmitter;

impl EventEmitter for NoopEventEmitter {
    fn emit(&self, _event: SessionEvent) -> Result<(), AgentError> {
        Ok(())
    }

    fn subscribe(&self) -> EventStream {
        let (sender, receiver) = unbounded();
        drop(sender);
        receiver
    }
}

#[derive(Default)]
struct BufferedState {
    events: Vec<SessionEvent>,
    subscribers: Vec<UnboundedSender<SessionEvent>>,
}

#[derive(Clone, Default)]
pub struct BufferedEventEmitter {
    inner: Arc<Mutex<BufferedState>>,
}

impl BufferedEventEmitter {
    pub fn snapshot(&self) -> Vec<SessionEvent> {
        let guard = self.inner.lock().expect("buffered emitter mutex poisoned");
        guard.events.clone()
    }
}

impl EventEmitter for BufferedEventEmitter {
    fn emit(&self, event: SessionEvent) -> Result<(), AgentError> {
        let mut guard = self.inner.lock().expect("buffered emitter mutex poisoned");
        guard.events.push(event.clone());

        let mut active_subscribers = Vec::with_capacity(guard.subscribers.len());
        for subscriber in guard.subscribers.drain(..) {
            if subscriber.unbounded_send(event.clone()).is_ok() {
                active_subscribers.push(subscriber);
            }
        }
        guard.subscribers = active_subscribers;
        Ok(())
    }

    fn subscribe(&self) -> EventStream {
        let (sender, receiver) = unbounded();
        let mut guard = self.inner.lock().expect("buffered emitter mutex poisoned");
        for event in &guard.events {
            if sender.unbounded_send(event.clone()).is_err() {
                return receiver;
            }
        }
        guard.subscribers.push(sender);
        receiver
    }
}

fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, executor::block_on};
    use serde_json::json;

    #[test]
    fn buffered_event_emitter_stores_emitted_events() {
        let emitter = BufferedEventEmitter::default();
        emitter
            .emit(SessionEvent::with_timestamp(
                EventKind::SessionStart,
                "2026-02-09T00:00:00Z",
                "s1",
                EventData::new(),
            ))
            .expect("emit should succeed");

        let events = emitter.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::SessionStart);
    }

    #[test]
    fn buffered_event_emitter_streams_events_to_subscribers() {
        let emitter = BufferedEventEmitter::default();
        let mut stream = emitter.subscribe();
        let event = SessionEvent::with_timestamp(
            EventKind::ToolCallStart,
            "2026-02-09T00:00:01Z",
            "s1",
            EventData::from_serializable(json!({
                "tool_name": "shell",
                "call_id": "c1"
            }))
            .expect("json object should convert to event data"),
        );

        emitter
            .emit(event.clone())
            .expect("emit should succeed with subscriber");

        let received = block_on(stream.next()).expect("subscriber should receive an event");
        assert_eq!(received, event);
    }

    #[test]
    fn event_kind_serializes_to_spec_names() {
        let serialized = serde_json::to_string(&EventKind::AssistantTextDelta).unwrap_or_default();
        assert_eq!(serialized, "\"ASSISTANT_TEXT_DELTA\"");
    }

    #[test]
    fn session_event_helpers_produce_expected_payload_shape() {
        let end = SessionEvent::tool_call_end_error("s1", "call-1", "tool failed");
        assert_eq!(end.kind, EventKind::ToolCallEnd);
        assert_eq!(end.data.get_str("call_id"), Some("call-1"));
        assert_eq!(end.data.get_str("error"), Some("tool failed"));

        let text = SessionEvent::assistant_text_end("s1", "done", Some("analysis"));
        assert_eq!(text.data.get_str("text"), Some("done"));
        assert_eq!(text.data.get_str("reasoning"), Some("analysis"));

        let warning = SessionEvent::context_usage_warning("s1", 100, 128_000, 81);
        assert_eq!(warning.kind, EventKind::Warning);
        assert_eq!(warning.data.get_str("severity"), Some("warning"));
        assert_eq!(warning.data.get_str("category"), Some("context_usage"));
    }

    #[test]
    fn event_data_rejects_non_object_serialization() {
        let err = EventData::from_serializable("not-an-object").expect_err("should reject scalar");
        assert!(matches!(
            err,
            AgentError::Session(SessionError::EventSerialization(_))
        ));
    }
}
