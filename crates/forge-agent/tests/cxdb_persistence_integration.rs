mod support;

use async_trait::async_trait;
use forge_agent::{CxdbPersistenceMode, LocalExecutionEnvironment, Session, SessionConfig};
use forge_turnstore_cxdb::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, HttpStoredTurn, MockCxdb,
};
use std::sync::Arc;
use support::{all_fixtures, client_with_adapter, enqueue, text_response};
use tempfile::tempdir;

#[derive(Clone, Debug, Default)]
struct FailingCxdb;

#[async_trait]
impl CxdbBinaryClient for FailingCxdb {
    async fn ctx_create(&self, _base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced create failure".to_string(),
        ))
    }

    async fn ctx_fork(&self, _from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        Err(CxdbClientError::Backend("forced fork failure".to_string()))
    }

    async fn append_turn(
        &self,
        _request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced append failure".to_string(),
        ))
    }

    async fn get_head(&self, _context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        Err(CxdbClientError::Backend("forced head failure".to_string()))
    }

    async fn get_last(
        &self,
        _context_id: u64,
        _limit: usize,
        _include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced list failure".to_string()))
    }

    async fn put_blob(&self, _raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
        Err(CxdbClientError::Backend("forced blob failure".to_string()))
    }

    async fn get_blob(&self, _content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced blob failure".to_string()))
    }

    async fn attach_fs(
        &self,
        _turn_id: u64,
        _fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        Err(CxdbClientError::Backend("forced fs failure".to_string()))
    }
}

#[async_trait]
impl CxdbHttpClient for FailingCxdb {
    async fn list_turns(
        &self,
        _context_id: u64,
        _before_turn_id: Option<u64>,
        _limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced list failure".to_string()))
    }

    async fn publish_registry_bundle(
        &self,
        _bundle_id: &str,
        _bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced registry failure".to_string(),
        ))
    }

    async fn get_registry_bundle(
        &self,
        _bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced registry failure".to_string(),
        ))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_mock_required_mode_persists_queryable_turns() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let backend = Arc::new(MockCxdb::default());
        let mut config = SessionConfig::default();
        config.cxdb_persistence = CxdbPersistenceMode::Required;
        let mut session = Session::new_with_cxdb_persistence(
            profile,
            env,
            client,
            config,
            backend.clone(),
            backend.clone(),
        )
        .expect("session should initialize");

        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-1", "done"),
        );
        session
            .submit("run once")
            .await
            .expect("submit should succeed");
        session.close().expect("close should succeed");

        let snapshot = session
            .persistence_snapshot()
            .await
            .expect("snapshot should succeed");
        let context_id = snapshot.context_id.expect("context should exist");
        let turns = backend
            .list_turns(context_id.parse::<u64>().expect("u64 context id"), None, 64)
            .await
            .expect("turns should be queryable");
        assert!(!turns.is_empty());
        assert!(
            turns
                .iter()
                .any(|turn| turn.type_id == "forge.agent.user_turn")
        );
        assert!(
            turns
                .iter()
                .any(|turn| turn.type_id == "forge.agent.assistant_turn")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_mode_off_does_not_touch_failing_backend() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut config = SessionConfig::default();
        config.cxdb_persistence = CxdbPersistenceMode::Off;
        let mut session = Session::new_with_cxdb_persistence(
            profile,
            env,
            client,
            config,
            Arc::new(FailingCxdb),
            Arc::new(FailingCxdb),
        )
        .expect("session should initialize");

        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-1", "done"),
        );
        session
            .submit("off mode")
            .await
            .expect("submit should succeed");
        session.close().expect("close should succeed");
    }
}
