mod support;

use forge_agent::{
    BufferedEventEmitter, CxdbPersistenceMode, EventKind, ExecutionEnvironment,
    LocalExecutionEnvironment, Session, SessionConfig, SessionPersistenceWriter, SessionState,
    Turn,
};
use forge_cxdb_runtime::{
    CxdbAppendTurnRequest, CxdbClientError, CxdbStoreContext, CxdbStoredTurn, CxdbStoredTurnRef,
    CxdbTurnId,
};
use forge_llm::Role;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use support::{
    all_fixtures, client_with_adapter, enqueue, text_response, tool_call_response,
    tool_result_by_call_id,
};
use tempfile::tempdir;

#[derive(serde::Deserialize)]
struct PersistedEnvelope {
    payload: serde_json::Value,
}

fn decode_persisted_envelope(payload: &[u8]) -> Option<PersistedEnvelope> {
    let tagged: BTreeMap<u64, serde_json::Value> = rmp_serde::from_slice(payload).ok()?;
    let payload_json = tagged.get(&8).and_then(serde_json::Value::as_str)?;
    let payload = serde_json::from_str::<serde_json::Value>(payload_json).ok()?;
    Some(PersistedEnvelope { payload })
}

#[derive(Default)]
struct RecordingPersistence {
    next_context: Mutex<u64>,
    next_turn: Mutex<u64>,
    appended: Mutex<Vec<CxdbAppendTurnRequest>>,
}

impl RecordingPersistence {
    fn appended(&self) -> Vec<CxdbAppendTurnRequest> {
        self.appended
            .lock()
            .expect("append mutex should lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl SessionPersistenceWriter for RecordingPersistence {
    async fn create_context(
        &self,
        _base_turn_id: Option<CxdbTurnId>,
    ) -> Result<CxdbStoreContext, CxdbClientError> {
        let mut next = self.next_context.lock().expect("context mutex should lock");
        if *next == 0 {
            *next = 1;
        }
        let context_id = next.to_string();
        *next += 1;
        Ok(CxdbStoreContext {
            context_id,
            head_turn_id: "0".to_string(),
            head_depth: 0,
        })
    }

    async fn append_turn(
        &self,
        request: CxdbAppendTurnRequest,
    ) -> Result<CxdbStoredTurn, CxdbClientError> {
        self.appended
            .lock()
            .expect("append mutex should lock")
            .push(request.clone());
        let mut next = self.next_turn.lock().expect("turn mutex should lock");
        if *next == 0 {
            *next = 1;
        }
        let turn_id = next.to_string();
        *next += 1;
        Ok(CxdbStoredTurn {
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

    async fn get_head(&self, context_id: &String) -> Result<CxdbStoredTurnRef, CxdbClientError> {
        Ok(CxdbStoredTurnRef {
            context_id: context_id.clone(),
            turn_id: "1".to_string(),
            depth: 1,
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_reasoning_effort_change_applies_on_next_request() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env, client, SessionConfig::default())
            .expect("session should initialize");

        session
            .set_reasoning_effort(Some("low".to_string()))
            .expect("low reasoning should be accepted");
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-1", "first done"),
        );
        session
            .submit("first input")
            .await
            .expect("first submit should succeed");

        session
            .set_reasoning_effort(Some("high".to_string()))
            .expect("high reasoning should be accepted");
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "second done"),
        );
        session
            .submit("second input")
            .await
            .expect("second submit should succeed");

        let seen = requests.lock().expect("requests mutex").clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].reasoning_effort.as_deref(), Some("low"));
        assert_eq!(seen[1].reasoning_effort.as_deref(), Some("high"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_steering_message_is_injected_before_next_tool_round() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("note.txt", "hello\n")
            .await
            .expect("seed file should write");

        let (client, responses, requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env, client, SessionConfig::default())
            .expect("session should initialize");

        session
            .steer("Please continue with concise output.")
            .expect("steering should be queued");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![(
                    "call-read",
                    "read_file",
                    json!({ "file_path": "note.txt", "offset": 1, "limit": 20 }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "done"),
        );

        session
            .submit("read file then finish")
            .await
            .expect("submit should succeed");

        let seen = requests.lock().expect("requests mutex").clone();
        assert_eq!(seen.len(), 2, "expected two model requests");
        assert!(
            seen[1].messages.iter().any(|message| {
                message.role == Role::User
                    && message
                        .text()
                        .contains("Please continue with concise output.")
            }),
            "expected steering user turn in second request for {}",
            fixture.id()
        );
        assert!(matches!(
            session
                .history()
                .iter()
                .find(|turn| matches!(turn, Turn::Steering(_))),
            Some(Turn::Steering(_))
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_loop_detection_warning_behavior() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("loop.txt", "x\n")
            .await
            .expect("seed file should write");

        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut config = SessionConfig::default();
        config.loop_detection_window = 4;
        let emitter = Arc::new(BufferedEventEmitter::default());
        let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
            .expect("session should initialize");

        for idx in 1..=4 {
            enqueue(
                &responses,
                tool_call_response(
                    fixture.id(),
                    fixture.model(),
                    &format!("resp-{idx}"),
                    vec![(
                        &format!("call-loop-{idx}"),
                        "read_file",
                        json!({ "file_path": "loop.txt", "offset": 1, "limit": 5 }),
                    )],
                ),
            );
        }
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-5", "loop broken"),
        );

        session
            .submit("loop scenario")
            .await
            .expect("submit should succeed");

        assert!(session.history().iter().any(|turn| {
            matches!(turn, Turn::Steering(turn) if turn.content.contains("Loop detected"))
        }));
        assert!(
            emitter
                .snapshot()
                .iter()
                .any(|event| event.kind == EventKind::LoopDetection)
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_error_recovery_after_tool_failure() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("recover.txt", "ready\n")
            .await
            .expect("seed file should write");

        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env, client, SessionConfig::default())
            .expect("session should initialize");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![("call-bad", "missing_tool", json!({ "x": 1 }))],
            ),
        );
        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-2",
                vec![(
                    "call-good",
                    "read_file",
                    json!({ "file_path": "recover.txt", "offset": 1, "limit": 20 }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-3", "recovered"),
        );

        session
            .submit("recover after failure")
            .await
            .expect("submit should succeed");

        let bad = tool_result_by_call_id(session.history(), "call-bad")
            .expect("bad call result should exist");
        assert!(bad.is_error);
        assert!(
            bad.content
                .as_str()
                .unwrap_or_default()
                .contains("Unknown tool")
        );

        let good = tool_result_by_call_id(session.history(), "call-good")
            .expect("good call result should exist");
        assert!(!good.is_error);
        assert!(good.content.as_str().unwrap_or_default().contains("ready"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_subagent_depth_limit_returns_tool_error() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut config = SessionConfig::default();
        config.max_subagent_depth = 0;
        let mut session =
            Session::new(profile, env, client, config).expect("session should initialize");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![(
                    "call-spawn",
                    "spawn_agent",
                    json!({ "task": "attempt nested task" }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "depth handled"),
        );

        session
            .submit("spawn too deep")
            .await
            .expect("submit should succeed");

        let result = tool_result_by_call_id(session.history(), "call-spawn")
            .expect("spawn result should exist");
        assert!(result.is_error);
        assert!(
            result
                .content
                .as_str()
                .unwrap_or_default()
                .contains("max_subagent_depth")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_question_response_transitions_awaiting_input_state() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env, client, SessionConfig::default())
            .expect("session should initialize");

        enqueue(
            &responses,
            text_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                "Which file should I update?",
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "Thanks."),
        );

        session
            .submit("start")
            .await
            .expect("first submit should succeed");
        assert_eq!(
            session.state(),
            &SessionState::AwaitingInput,
            "expected awaiting-input state for {}",
            fixture.id()
        );

        session
            .submit("Update src/lib.rs")
            .await
            .expect("second submit should succeed");
        assert_eq!(
            session.state(),
            &SessionState::Idle,
            "expected idle state after answer for {}",
            fixture.id()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_large_output_truncation_behavior() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("big.txt", &"x".repeat(2_000))
            .await
            .expect("seed file should write");

        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut config = SessionConfig::default();
        config
            .tool_output_limits
            .insert("read_file".to_string(), 80);
        let mut session =
            Session::new(profile, env, client, config).expect("session should initialize");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![(
                    "call-read-big",
                    "read_file",
                    json!({ "file_path": "big.txt", "offset": 1, "limit": 2000 }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "truncated"),
        );

        session.submit("read big file").await.expect("submit");
        let result = tool_result_by_call_id(session.history(), "call-read-big")
            .expect("result should exist");
        let text = result.content.as_str().unwrap_or_default();
        assert!(text.contains("[WARNING: Tool output was truncated."));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_subagent_spawn_persists_link_record() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let store = Arc::new(RecordingPersistence::default());
        let mut config = SessionConfig::default();
        config.cxdb_persistence = CxdbPersistenceMode::Required;
        let mut session =
            Session::new_with_persistence(profile, env, client, config, Some(store.clone()))
                .expect("session should initialize");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![(
                    "call-spawn",
                    "spawn_agent",
                    json!({ "task": "child task", "max_turns": 1 }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "done"),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-3", "child-done"),
        );

        session
            .submit("spawn child")
            .await
            .expect("submit should succeed");

        let appended = store.appended();
        let spawn_links: Vec<PersistedEnvelope> = appended
            .iter()
            .filter(|request| request.type_id == "forge.link.subagent_spawn")
            .filter_map(|request| decode_persisted_envelope(&request.payload))
            .collect();

        assert!(
            !spawn_links.is_empty(),
            "expected subagent spawn link turn for {}",
            fixture.id()
        );
        let payload = &spawn_links[0].payload;
        assert_eq!(payload["session_id"].as_str(), Some(session.id()));
        assert!(payload["parent_turn"].as_str().is_some());
        assert!(payload["child_context_id"].as_str().is_some());
        assert_eq!(payload["thread_key"].as_str(), session.thread_key());
    }
}
