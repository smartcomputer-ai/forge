use async_trait::async_trait;
use forge_attractor::{
    AttractorError, Graph, Node, NodeExecutor, NodeOutcome, NodeStatus, PipelineEvent,
    PipelineRunner, RunConfig, RuntimeContext, RuntimeEventKind, StageEvent, parse_dot,
    runtime_event_channel,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct RetryOnceExecutor {
    attempts: AtomicUsize,
}

#[async_trait]
impl NodeExecutor for RetryOnceExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        if node.id == "work" && self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(NodeOutcome {
                status: NodeStatus::Retry,
                notes: Some("retry please".to_string()),
                context_updates: RuntimeContext::new(),
                preferred_label: None,
                suggested_next_ids: Vec::new(),
            });
        }
        Ok(NodeOutcome::success())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn event_stream_retry_flow_expected_ordered_sequence_and_retrying_event() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [shape=box, max_retries=1]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
    )
    .expect("graph should parse");

    let (tx, mut rx) = runtime_event_channel();
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                events: forge_attractor::RuntimeEventSink::with_sender(tx),
                executor: Arc::new(RetryOnceExecutor {
                    attempts: AtomicUsize::new(0),
                }),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");
    assert_eq!(result.status, forge_attractor::PipelineStatus::Success);

    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert!(events.len() >= 8, "expected enough events for retry flow");

    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.sequence_no as usize, index + 1);
    }

    assert!(matches!(
        events.first().map(|event| &event.kind),
        Some(RuntimeEventKind::Pipeline(PipelineEvent::Started { .. }))
    ));
    assert!(matches!(
        events.last().map(|event| &event.kind),
        Some(RuntimeEventKind::Pipeline(PipelineEvent::Completed { .. }))
    ));
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            RuntimeEventKind::Stage(StageEvent::Retrying {
                node_id: ref node,
                attempt: 1,
                next_attempt: 2,
                ..
            }) if node == "work"
        )
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn event_payload_shape_expected_category_and_kind_tags() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            exit [shape=Msquare]
            start -> exit
        }
        "#,
    )
    .expect("graph should parse");

    let (tx, mut rx) = runtime_event_channel();
    PipelineRunner
        .run(
            &graph,
            RunConfig {
                events: forge_attractor::RuntimeEventSink::with_sender(tx),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    let event = rx.try_recv().expect("expected first event");
    let encoded = serde_json::to_value(&event).expect("event should serialize");
    assert_eq!(
        encoded.get("kind").and_then(|kind| kind.get("category")),
        Some(&serde_json::json!("pipeline"))
    );
    assert_eq!(
        encoded
            .get("kind")
            .and_then(|kind| kind.get("kind"))
            .and_then(serde_json::Value::as_str),
        Some("started")
    );
}
