mod support;

use async_trait::async_trait;
use forge_agent::{
    LocalExecutionEnvironment, Session, SessionConfig, TurnStoreWriteMode,
};
use forge_turnstore::{
    AppendTurnRequest, ContextId, FsTurnStore, MemoryTurnStore, StoreContext, StoredTurn,
    StoredTurnEnvelope, StoredTurnRef, TurnId, TurnStore, TurnStoreError,
};
use std::sync::{Arc, Mutex};
use support::{all_fixtures, client_with_adapter, enqueue, text_response};
use tempfile::tempdir;

#[derive(Default)]
struct CountingTurnStore {
    create_calls: Mutex<u64>,
    append_calls: Mutex<u64>,
    next_context: Mutex<u64>,
    next_turn: Mutex<u64>,
}

impl CountingTurnStore {
    fn counts(&self) -> (u64, u64) {
        (
            *self.create_calls.lock().expect("create mutex should lock"),
            *self.append_calls.lock().expect("append mutex should lock"),
        )
    }
}

#[async_trait]
impl TurnStore for CountingTurnStore {
    async fn create_context(
        &self,
        _base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        *self.create_calls.lock().expect("create mutex should lock") += 1;
        let mut next = self.next_context.lock().expect("context mutex should lock");
        if *next == 0 {
            *next = 1;
        }
        let context_id = next.to_string();
        *next += 1;
        Ok(StoreContext {
            context_id,
            head_turn_id: "0".to_string(),
            head_depth: 0,
        })
    }

    async fn append_turn(&self, request: AppendTurnRequest) -> Result<StoredTurn, TurnStoreError> {
        *self.append_calls.lock().expect("append mutex should lock") += 1;
        let mut next = self.next_turn.lock().expect("turn mutex should lock");
        if *next == 0 {
            *next = 1;
        }
        let turn_id = next.to_string();
        *next += 1;
        Ok(StoredTurn {
            context_id: request.context_id,
            turn_id,
            parent_turn_id: request.parent_turn_id.unwrap_or_else(|| "0".to_string()),
            depth: 1,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: Some(request.idempotency_key),
            content_hash: None,
        })
    }

    async fn fork_context(&self, from_turn_id: TurnId) -> Result<StoreContext, TurnStoreError> {
        Ok(StoreContext {
            context_id: "fork".to_string(),
            head_turn_id: from_turn_id,
            head_depth: 1,
        })
    }

    async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, TurnStoreError> {
        Ok(StoredTurnRef {
            context_id: context_id.clone(),
            turn_id: "0".to_string(),
            depth: 0,
        })
    }

    async fn list_turns(
        &self,
        _context_id: &ContextId,
        _before_turn_id: Option<&TurnId>,
        _limit: usize,
    ) -> Result<Vec<StoredTurn>, TurnStoreError> {
        Ok(Vec::new())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn turnstore_memory_required_mode_persists_queryable_turns() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let store = Arc::new(MemoryTurnStore::new());
        let mut config = SessionConfig::default();
        config.turn_store_mode = TurnStoreWriteMode::Required;
        let mut session =
            Session::new_with_turn_store(profile, env, client, config, Some(store.clone()))
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

        let turns = store
            .list_turns(&"1".to_string(), None, 64)
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
async fn turnstore_fs_required_mode_persists_queryable_turns_after_reopen() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let store_dir = tempdir().expect("store dir should be created");
        let store =
            Arc::new(FsTurnStore::new(store_dir.path()).expect("fs store should initialize"));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut config = SessionConfig::default();
        config.turn_store_mode = TurnStoreWriteMode::Required;
        let mut session = Session::new_with_turn_store(profile, env, client, config, Some(store))
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
        drop(session);

        let reopened = FsTurnStore::new(store_dir.path()).expect("fs store should reopen");
        let turns = reopened
            .list_turns(&"1".to_string(), None, 64)
            .await
            .expect("turns should be queryable after reopen");
        assert!(!turns.is_empty());

        let envelopes: Vec<StoredTurnEnvelope> = turns
            .iter()
            .filter_map(|turn| serde_json::from_slice::<StoredTurnEnvelope>(&turn.payload).ok())
            .collect();
        assert!(
            envelopes
                .iter()
                .any(|envelope| envelope.event_kind == "session_start"),
            "expected session_start event in persisted envelopes for {}",
            fixture.id()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn turnstore_mode_off_does_not_call_store() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let counting = Arc::new(CountingTurnStore::default());
        let mut config = SessionConfig::default();
        config.turn_store_mode = TurnStoreWriteMode::Off;
        let mut session =
            Session::new_with_turn_store(profile, env, client, config, Some(counting.clone()))
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

        let (create_calls, append_calls) = counting.counts();
        assert_eq!(create_calls, 0, "off mode should not create contexts");
        assert_eq!(append_calls, 0, "off mode should not append turns");
    }
}
