use forge_turnstore::{
    AppendTurnRequest, ArtifactStore, FsTurnStore, MemoryTurnStore, TurnStore, TurnStoreResult,
};

fn append_request(context_id: &str, payload: &[u8], key: &str) -> AppendTurnRequest {
    AppendTurnRequest {
        context_id: context_id.to_string(),
        parent_turn_id: None,
        type_id: "forge.agent.user_turn".to_string(),
        type_version: 1,
        payload: payload.to_vec(),
        idempotency_key: key.to_string(),
    }
}

async fn exercise_idempotent_append<T: TurnStore>(store: &T) -> TurnStoreResult<()> {
    let context = store.create_context(None).await?;
    let first = store
        .append_turn(append_request(&context.context_id, b"hello", "append-1"))
        .await?;
    let second = store
        .append_turn(append_request(&context.context_id, b"hello", "append-1"))
        .await?;
    assert_eq!(first.turn_id, second.turn_id);

    let turns = store.list_turns(&context.context_id, None, 10).await?;
    assert_eq!(turns.len(), 1);
    Ok(())
}

async fn exercise_fork_and_list<T: TurnStore>(store: &T) -> TurnStoreResult<()> {
    let root = store.create_context(None).await?;
    let t1 = store
        .append_turn(append_request(&root.context_id, b"turn-1", "k1"))
        .await?;
    let t2 = store
        .append_turn(append_request(&root.context_id, b"turn-2", "k2"))
        .await?;

    let fork = store.fork_context(t1.turn_id.clone()).await?;
    let fork_head = store.get_head(&fork.context_id).await?;
    assert_eq!(fork_head.turn_id, t1.turn_id);

    let root_turns = store.list_turns(&root.context_id, None, 10).await?;
    assert_eq!(root_turns.len(), 2);
    assert_eq!(root_turns[0].turn_id, t1.turn_id);
    assert_eq!(root_turns[1].turn_id, t2.turn_id);

    let older = store
        .list_turns(&root.context_id, Some(&t2.turn_id), 10)
        .await?;
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].turn_id, t1.turn_id);
    Ok(())
}

async fn exercise_artifact_roundtrip<T: TurnStore + ArtifactStore>(store: &T) -> TurnStoreResult<()> {
    let context = store.create_context(None).await?;
    let turn = store
        .append_turn(append_request(&context.context_id, b"payload", "artifact-k1"))
        .await?;

    let blob = b"immutable-blob-bytes";
    let hash = store.put_blob(blob).await?;
    let fetched = store.get_blob(&hash).await?;
    assert_eq!(fetched.as_deref(), Some(blob.as_slice()));

    store.attach_fs(&turn.turn_id, &hash).await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn memory_store_idempotent_append_expected_single_turn() {
    let store = MemoryTurnStore::new();
    exercise_idempotent_append(&store)
        .await
        .expect("memory idempotent append should succeed");
}

#[tokio::test(flavor = "current_thread")]
async fn fs_store_idempotent_append_expected_single_turn() {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let store = FsTurnStore::new(tmp.path()).expect("fs store should initialize");
    exercise_idempotent_append(&store)
        .await
        .expect("fs idempotent append should succeed");
}

#[tokio::test(flavor = "current_thread")]
async fn memory_and_fs_fork_list_expected_same_behavior() {
    let memory = MemoryTurnStore::new();
    exercise_fork_and_list(&memory)
        .await
        .expect("memory fork/list should succeed");

    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let fs = FsTurnStore::new(tmp.path()).expect("fs store should initialize");
    exercise_fork_and_list(&fs)
        .await
        .expect("fs fork/list should succeed");
}

#[tokio::test(flavor = "current_thread")]
async fn memory_and_fs_artifact_store_expected_same_behavior() {
    let memory = MemoryTurnStore::new();
    exercise_artifact_roundtrip(&memory)
        .await
        .expect("memory artifact roundtrip should succeed");

    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let fs = FsTurnStore::new(tmp.path()).expect("fs store should initialize");
    exercise_artifact_roundtrip(&fs)
        .await
        .expect("fs artifact roundtrip should succeed");
}
