use forge_attractor::{
    AttractorStageToAgentLinkRecord, AttractorStorageReader, AttractorStorageWriter, ContextId,
    CxdbPersistenceMode, PipelineRunner, PipelineStatus, RunConfig, attractor_idempotency_key,
    parse_dot, query_latest_checkpoint_snapshot, query_run_metadata, query_stage_timeline,
    query_stage_to_agent_linkage,
};
use forge_cxdb_runtime::{CxdbRuntimeStore, MockCxdb};
use std::sync::Arc;

#[derive(Clone)]
enum Harness {
    Cxdb(Arc<CxdbRuntimeStore<Arc<MockCxdb>, Arc<MockCxdb>>>),
}

impl Harness {
    fn writer(&self) -> Arc<dyn AttractorStorageWriter> {
        match self {
            Self::Cxdb(store) => store.clone(),
        }
    }

    fn reader(&self) -> Arc<dyn AttractorStorageReader> {
        match self {
            Self::Cxdb(store) => store.clone(),
        }
    }

    async fn append_stage_link(&self, context_id: &ContextId, run_id: &str) {
        let record = AttractorStageToAgentLinkRecord {
            timestamp: "1.000Z".to_string(),
            run_id: run_id.to_string(),
            pipeline_context_id: context_id.clone(),
            node_id: "plan".to_string(),
            stage_attempt_id: "plan:attempt:1".to_string(),
            agent_session_id: "session-1".to_string(),
            agent_context_id: "agent-ctx-1".to_string(),
            agent_head_turn_id: Some("42".to_string()),
            parent_turn_id: Some("7".to_string()),
            sequence_no: 999,
            thread_key: Some("main".to_string()),
        };
        let key =
            attractor_idempotency_key(run_id, "plan", "plan:attempt:1", "stage_to_agent_link", 999);
        match self {
            Self::Cxdb(store) => {
                store
                    .append_stage_to_agent_link(context_id, record, key)
                    .await
                    .expect("append link should succeed");
            }
        }
    }
}

fn graph_under_test() -> forge_attractor::Graph {
    parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    )
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn storage_queries_memory_and_fs_expected_parity() {
    let backend_a = Arc::new(MockCxdb::default());
    let backend_b = Arc::new(MockCxdb::default());
    let harnesses = [
        Harness::Cxdb(Arc::new(CxdbRuntimeStore::new(
            backend_a.clone(),
            backend_a.clone(),
        ))),
        Harness::Cxdb(Arc::new(CxdbRuntimeStore::new(
            backend_b.clone(),
            backend_b.clone(),
        ))),
    ];

    let mut stage_event_kinds_by_backend = Vec::new();
    for harness in harnesses {
        let result = PipelineRunner
            .run(
                &graph_under_test(),
                RunConfig {
                    run_id: Some("run-q".to_string()),
                    storage: Some(harness.writer()),
                    cxdb_persistence: CxdbPersistenceMode::Required,
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert_eq!(result.status, PipelineStatus::Success);

        let context_id = "1".to_string();
        harness.append_stage_link(&context_id, "run-q").await;

        let metadata = query_run_metadata(&*harness.reader(), &context_id)
            .await
            .expect("run metadata query should succeed");
        assert_eq!(metadata.run_id.as_deref(), Some("run-q"));
        assert_eq!(metadata.status.as_deref(), Some("success"));
        assert_eq!(metadata.graph_id.as_deref(), Some("G"));

        let timeline = query_stage_timeline(&*harness.reader(), &context_id)
            .await
            .expect("stage timeline query should succeed");
        assert!(!timeline.is_empty());
        let event_kinds: Vec<String> = timeline
            .iter()
            .map(|entry| entry.event_kind.clone())
            .collect();
        assert!(event_kinds.iter().any(|kind| kind == "stage_started"));
        assert!(event_kinds.iter().any(|kind| kind == "stage_completed"));

        let checkpoint = query_latest_checkpoint_snapshot(&*harness.reader(), &context_id)
            .await
            .expect("checkpoint query should succeed")
            .expect("checkpoint snapshot should exist");
        assert!(checkpoint.checkpoint_id.starts_with("cp-"));
        assert!(checkpoint.state_summary.get("current_node_id").is_some());

        let links = query_stage_to_agent_linkage(&*harness.reader(), &context_id)
            .await
            .expect("stage-link query should succeed");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].node_id, "plan");

        stage_event_kinds_by_backend.push(event_kinds);
    }

    assert_eq!(
        stage_event_kinds_by_backend[0],
        stage_event_kinds_by_backend[1]
    );
}
