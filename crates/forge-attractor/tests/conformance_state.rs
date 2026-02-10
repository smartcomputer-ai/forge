use forge_attractor::{
    AttractorStorageWriter, CxdbPersistenceMode, PipelineRunner, PipelineStatus, RunConfig,
    parse_dot,
};
use forge_cxdb_runtime::{CxdbRuntimeStore, MockCxdb};
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Clone)]
enum StorageHarness {
    Cxdb(Arc<CxdbRuntimeStore<Arc<MockCxdb>, Arc<MockCxdb>>>),
}

impl StorageHarness {
    fn writer(&self) -> Arc<dyn AttractorStorageWriter> {
        match self {
            Self::Cxdb(store) => store.clone(),
        }
    }
}

fn linear_graph() -> forge_attractor::Graph {
    parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            a [shape=box, prompt="A"]
            b [shape=box, prompt="B"]
            exit [shape=Msquare]
            start -> a -> b -> exit
        }
        "#,
    )
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn conformance_state_memory_and_fs_expected_checkpoint_and_resume_parity() {
    let backend_a = Arc::new(MockCxdb::default());
    let backend_b = Arc::new(MockCxdb::default());
    let harnesses = vec![
        StorageHarness::Cxdb(Arc::new(CxdbRuntimeStore::new(
            backend_a.clone(),
            backend_a.clone(),
        ))),
        StorageHarness::Cxdb(Arc::new(CxdbRuntimeStore::new(
            backend_b.clone(),
            backend_b.clone(),
        ))),
    ];

    for harness in harnesses {
        let logs_root = TempDir::new().expect("tempdir should create");
        let first = PipelineRunner
            .run(
                &linear_graph(),
                RunConfig {
                    run_id: Some("conformance-state".to_string()),
                    logs_root: Some(logs_root.path().to_path_buf()),
                    storage: Some(harness.writer()),
                    cxdb_persistence: CxdbPersistenceMode::Required,
                    ..RunConfig::default()
                },
            )
            .await
            .expect("first run should succeed");
        assert_eq!(first.status, PipelineStatus::Success);

        let checkpoint = logs_root.path().join("checkpoint.json");
        assert!(checkpoint.exists());

        let resumed = PipelineRunner
            .run(
                &linear_graph(),
                RunConfig {
                    run_id: Some("conformance-state".to_string()),
                    logs_root: Some(logs_root.path().to_path_buf()),
                    resume_from_checkpoint: Some(checkpoint),
                    storage: Some(harness.writer()),
                    cxdb_persistence: CxdbPersistenceMode::Required,
                    ..RunConfig::default()
                },
            )
            .await
            .expect("resume run should succeed");

        assert_eq!(resumed.status, PipelineStatus::Success);
        assert_eq!(resumed.completed_nodes, first.completed_nodes);
    }
}
