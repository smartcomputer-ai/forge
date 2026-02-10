use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub sequence_no: u64,
    pub timestamp: String,
    pub kind: RuntimeEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "snake_case")]
pub enum RuntimeEventKind {
    Pipeline(PipelineEvent),
    Stage(StageEvent),
    Parallel(ParallelEvent),
    Interview(InterviewEvent),
    Checkpoint(CheckpointEvent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PipelineEvent {
    Started {
        run_id: String,
        graph_id: String,
        lineage_attempt: u32,
    },
    Resumed {
        run_id: String,
        graph_id: String,
        lineage_attempt: u32,
    },
    Completed {
        run_id: String,
        graph_id: String,
        lineage_attempt: u32,
    },
    Failed {
        run_id: String,
        graph_id: String,
        lineage_attempt: u32,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StageEvent {
    Started {
        run_id: String,
        node_id: String,
        stage_attempt_id: String,
        attempt: u32,
    },
    Completed {
        run_id: String,
        node_id: String,
        stage_attempt_id: String,
        attempt: u32,
        status: String,
        notes: Option<String>,
    },
    Failed {
        run_id: String,
        node_id: String,
        stage_attempt_id: String,
        attempt: u32,
        status: String,
        notes: Option<String>,
        will_retry: bool,
    },
    Retrying {
        run_id: String,
        node_id: String,
        stage_attempt_id: String,
        attempt: u32,
        next_attempt: u32,
        delay_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParallelEvent {
    Started {
        run_id: String,
        node_id: String,
        branch_count: usize,
    },
    BranchStarted {
        run_id: String,
        node_id: String,
        branch_id: String,
        branch_index: usize,
        target_node: String,
    },
    BranchCompleted {
        run_id: String,
        node_id: String,
        branch_id: String,
        branch_index: usize,
        target_node: String,
        status: String,
        notes: Option<String>,
    },
    Completed {
        run_id: String,
        node_id: String,
        success_count: usize,
        failure_count: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterviewEvent {
    Started {
        run_id: String,
        node_id: String,
    },
    Completed {
        run_id: String,
        node_id: String,
        selected: Option<String>,
    },
    Timeout {
        run_id: String,
        node_id: String,
        default_selected: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckpointEvent {
    Saved {
        run_id: String,
        node_id: String,
        checkpoint_id: String,
    },
}

pub trait RuntimeEventObserver: Send + Sync {
    fn on_event(&self, event: &RuntimeEvent);
}

impl<F> RuntimeEventObserver for F
where
    F: Fn(&RuntimeEvent) + Send + Sync,
{
    fn on_event(&self, event: &RuntimeEvent) {
        self(event);
    }
}

pub type SharedRuntimeEventObserver = Arc<dyn RuntimeEventObserver>;
pub type RuntimeEventSender = mpsc::UnboundedSender<RuntimeEvent>;
pub type RuntimeEventReceiver = mpsc::UnboundedReceiver<RuntimeEvent>;

#[derive(Clone, Default)]
pub struct RuntimeEventSink {
    observer: Option<SharedRuntimeEventObserver>,
    sender: Option<RuntimeEventSender>,
}

impl RuntimeEventSink {
    pub fn with_observer(observer: SharedRuntimeEventObserver) -> Self {
        Self {
            observer: Some(observer),
            sender: None,
        }
    }

    pub fn with_sender(sender: RuntimeEventSender) -> Self {
        Self {
            observer: None,
            sender: Some(sender),
        }
    }

    pub fn observer(mut self, observer: SharedRuntimeEventObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn sender(mut self, sender: RuntimeEventSender) -> Self {
        self.sender = Some(sender);
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.observer.is_some() || self.sender.is_some()
    }

    pub fn emit(&self, event: RuntimeEvent) {
        if let Some(observer) = self.observer.as_ref() {
            observer.on_event(&event);
        }
        if let Some(sender) = self.sender.as_ref() {
            let _ = sender.send(event);
        }
    }
}

pub fn runtime_event_channel() -> (RuntimeEventSender, RuntimeEventReceiver) {
    mpsc::unbounded_channel()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn runtime_event_sink_observer_and_sender_expected_both_receive_events() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer_seen = Arc::clone(&seen);
        let observer: SharedRuntimeEventObserver = Arc::new(move |event: &RuntimeEvent| {
            observer_seen
                .lock()
                .expect("observer mutex should lock")
                .push(event.sequence_no);
        });
        let (tx, mut rx) = runtime_event_channel();
        let sink = RuntimeEventSink::with_observer(observer).sender(tx);
        sink.emit(RuntimeEvent {
            sequence_no: 7,
            timestamp: "1.000Z".to_string(),
            kind: RuntimeEventKind::Pipeline(PipelineEvent::Started {
                run_id: "run-1".to_string(),
                graph_id: "g".to_string(),
                lineage_attempt: 1,
            }),
        });

        let streamed = rx.try_recv().expect("channel should receive one event");
        assert_eq!(streamed.sequence_no, 7);
        assert_eq!(
            seen.lock().expect("observer mutex should lock").as_slice(),
            &[7]
        );
    }
}
