use crate::AgentError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type EventData = HashMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub kind: EventKind,
    pub timestamp: String,
    pub session_id: String,
    pub data: EventData,
}

pub trait EventEmitter: Send + Sync {
    fn emit(&self, event: SessionEvent) -> Result<(), AgentError>;
}

#[derive(Default)]
pub struct NoopEventEmitter;

impl EventEmitter for NoopEventEmitter {
    fn emit(&self, _event: SessionEvent) -> Result<(), AgentError> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct BufferedEventEmitter {
    inner: Arc<Mutex<Vec<SessionEvent>>>,
}

impl BufferedEventEmitter {
    pub fn snapshot(&self) -> Vec<SessionEvent> {
        let guard = self.inner.lock().expect("buffered emitter mutex poisoned");
        guard.clone()
    }
}

impl EventEmitter for BufferedEventEmitter {
    fn emit(&self, event: SessionEvent) -> Result<(), AgentError> {
        let mut guard = self.inner.lock().expect("buffered emitter mutex poisoned");
        guard.push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_event_emitter_stores_emitted_events() {
        let emitter = BufferedEventEmitter::default();
        emitter
            .emit(SessionEvent {
                kind: EventKind::SessionStart,
                timestamp: "2026-02-09T00:00:00Z".to_string(),
                session_id: "s1".to_string(),
                data: EventData::new(),
            })
            .expect("emit should succeed");

        let events = emitter.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::SessionStart);
    }
}
