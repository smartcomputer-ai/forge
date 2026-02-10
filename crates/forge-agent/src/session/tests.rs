
use super::*;
use crate::{
    BufferedEventEmitter, LocalExecutionEnvironment, PROJECT_DOC_TRUNCATION_MARKER,
    ProviderCapabilities, RegisteredTool, StaticProviderProfile, ToolCallHook, ToolExecutor,
    ToolPreHookOutcome, ToolRegistry, build_openai_tool_registry,
};
use async_trait::async_trait;
use forge_llm::{
    Client, ConfigurationError, ContentPart, FinishReason, Message, ProviderAdapter, Request,
    Response, Role, SDKError, StreamEventStream, ToolCallData, Usage,
};
use futures::{StreamExt, executor::block_on};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::tempdir;

#[derive(Clone)]
struct SequenceAdapter {
    responses: Arc<Mutex<VecDeque<Response>>>,
    requests: Arc<Mutex<Vec<Request>>>,
    delay_ms: u64,
}

#[async_trait]
impl ProviderAdapter for SequenceAdapter {
    fn name(&self) -> &str {
        "test"
    }

    async fn complete(&self, request: Request) -> Result<Response, SDKError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        self.requests.lock().expect("requests mutex").push(request);
        self.responses
            .lock()
            .expect("responses mutex")
            .pop_front()
            .ok_or_else(|| SDKError::Configuration(ConfigurationError::new("no response queued")))
    }

    async fn stream(&self, _request: Request) -> Result<StreamEventStream, SDKError> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

#[derive(Default)]
struct RecordingHook {
    pre_calls: Mutex<Vec<String>>,
    post_calls: Mutex<Vec<String>>,
    skip_tool_name: Option<String>,
}

#[derive(Default)]
struct RecordingPersistence {
    next_context_id: Mutex<u64>,
    next_turn_id: Mutex<u64>,
    append_requests: Mutex<Vec<CxdbAppendTurnRequest>>,
    snapshot_calls: Mutex<usize>,
    fail_create: bool,
    fail_append: bool,
}

impl RecordingPersistence {
    fn with_failures(fail_create: bool, fail_append: bool) -> Self {
        Self {
            next_context_id: Mutex::new(1),
            next_turn_id: Mutex::new(1),
            append_requests: Mutex::new(Vec::new()),
            snapshot_calls: Mutex::new(0),
            fail_create,
            fail_append,
        }
    }

    fn appended(&self) -> Vec<CxdbAppendTurnRequest> {
        self.append_requests
            .lock()
            .expect("append requests mutex")
            .clone()
    }
}

#[test]
fn typed_record_msgpack_roundtrip_preserves_payload_and_metadata() {
    let record = ToolCallLifecycleRecord {
        session_id: "session-1".to_string(),
        kind: "ended".to_string(),
        timestamp: "123.000Z".to_string(),
        call_id: "call-1".to_string(),
        tool_name: Some("echo_tool".to_string()),
        arguments: Some(serde_json::json!({"value":"hello"})),
        output: Some(serde_json::json!({"ok":true})),
        is_error: Some(false),
        sequence_no: 5,
        thread_key: Some("main".to_string()),
        fs_root_hash: Some("abc".to_string()),
        snapshot_policy_id: Some("default".to_string()),
        snapshot_stats: Some(FsSnapshotStatsRecord {
            file_count: 1,
            dir_count: 1,
            symlink_count: 0,
            total_bytes: 64,
            bytes_uploaded: 64,
        }),
    };

    let bytes = encode_typed_record("forge.agent.tool_call_lifecycle", &record)
        .expect("encode should succeed");
    let decoded: ToolCallLifecycleRecord =
        decode_typed_record(&bytes).expect("decode should succeed");
    assert_eq!(decoded, record);
}

#[async_trait]
impl SessionPersistenceWriter for RecordingPersistence {
    async fn create_context(
        &self,
        _base_turn_id: Option<CxdbTurnId>,
    ) -> Result<CxdbStoreContext, CxdbClientError> {
        if self.fail_create {
            return Err(CxdbClientError::Backend(
                "forced create failure".to_string(),
            ));
        }
        let mut next = self.next_context_id.lock().expect("next context mutex");
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
        if self.fail_append {
            return Err(CxdbClientError::Backend(
                "forced append failure".to_string(),
            ));
        }
        self.append_requests
            .lock()
            .expect("append requests mutex")
            .push(request.clone());
        let mut next = self.next_turn_id.lock().expect("next turn mutex");
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
            turn_id: "0".to_string(),
            depth: 0,
        })
    }

    async fn capture_upload_workspace(
        &self,
        _workspace_root: &Path,
        policy: &CxdbFsSnapshotPolicy,
    ) -> Result<CxdbFsSnapshotCapture, CxdbClientError> {
        let mut calls = self.snapshot_calls.lock().expect("snapshot calls mutex");
        *calls += 1;
        Ok(CxdbFsSnapshotCapture {
            fs_root_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            policy_id: policy.policy_id.clone(),
            stats: forge_cxdb_runtime::CxdbFsSnapshotStats {
                file_count: 2,
                dir_count: 1,
                symlink_count: 0,
                total_bytes: 64,
                bytes_uploaded: 64,
            },
        })
    }
}

#[async_trait]
impl ToolCallHook for RecordingHook {
    async fn before_tool_call(
        &self,
        context: &crate::ToolHookContext,
    ) -> Result<ToolPreHookOutcome, AgentError> {
        self.pre_calls
            .lock()
            .expect("pre hook mutex")
            .push(context.tool_name.clone());
        if self
            .skip_tool_name
            .as_deref()
            .is_some_and(|name| name == context.tool_name)
        {
            return Ok(ToolPreHookOutcome::Skip {
                message: format!("skipped {}", context.tool_name),
                is_error: true,
            });
        }
        Ok(ToolPreHookOutcome::Continue)
    }

    async fn after_tool_call(
        &self,
        context: &crate::ToolPostHookContext,
    ) -> Result<(), AgentError> {
        self.post_calls
            .lock()
            .expect("post hook mutex")
            .push(context.tool.tool_name.clone());
        Ok(())
    }
}

fn test_usage() -> Usage {
    Usage {
        input_tokens: 1,
        output_tokens: 1,
        total_tokens: 2,
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        raw: None,
    }
}

fn text_response(id: &str, text: &str) -> Response {
    Response {
        id: id.to_string(),
        model: "gpt-5.2-codex".to_string(),
        provider: "test".to_string(),
        message: Message::assistant(text),
        finish_reason: FinishReason {
            reason: "stop".to_string(),
            raw: None,
        },
        usage: test_usage(),
        raw: None,
        warnings: Vec::new(),
        rate_limit: None,
    }
}

fn tool_call_response(id: &str, call_id: &str, tool_name: &str, args: Value) -> Response {
    Response {
        id: id.to_string(),
        model: "gpt-5.2-codex".to_string(),
        provider: "test".to_string(),
        message: Message {
            role: forge_llm::Role::Assistant,
            content: vec![ContentPart::tool_call(ToolCallData {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                arguments: args,
                r#type: "function".to_string(),
            })],
            name: None,
            tool_call_id: None,
        },
        finish_reason: FinishReason {
            reason: "tool_calls".to_string(),
            raw: None,
        },
        usage: test_usage(),
        raw: None,
        warnings: Vec::new(),
        rate_limit: None,
    }
}

fn build_test_client(responses: Vec<Response>) -> (Arc<Client>, Arc<Mutex<Vec<Request>>>) {
    build_test_client_with_delay(responses, 0)
}

fn build_test_client_with_delay(
    responses: Vec<Response>,
    delay_ms: u64,
) -> (Arc<Client>, Arc<Mutex<Vec<Request>>>) {
    let adapter = Arc::new(SequenceAdapter {
        responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        requests: Arc::new(Mutex::new(Vec::new())),
        delay_ms,
    });

    let requests = adapter.requests.clone();
    let mut client = Client::default();
    client
        .register_provider(adapter)
        .expect("provider should register");
    (Arc::new(client), requests)
}

fn tool_registry_with_echo() -> Arc<ToolRegistry> {
    tool_registry_with_named_echoes(&["echo_tool"])
}

fn tool_registry_with_named_echoes(names: &[&str]) -> Arc<ToolRegistry> {
    let mut tool_registry = ToolRegistry::default();
    for name in names {
        let executor: ToolExecutor = Arc::new(|args, _env| {
            Box::pin(async move {
                let output = args
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("missing")
                    .to_string();
                Ok(output)
            })
        });
        tool_registry.register(RegisteredTool {
            definition: forge_llm::ToolDefinition {
                name: (*name).to_string(),
                description: "echo".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["value"],
                    "properties": {
                        "value": { "type": "string" }
                    }
                }),
            },
            executor,
        });
    }
    Arc::new(tool_registry)
}

fn write_test_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent directory should be created");
    }
    fs::write(path, content).expect("file should be written");
}

#[test]
fn session_new_emits_session_start() {
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let session = Session::new_with_emitter(
        profile,
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("session should initialize");

    assert!(!session.id().is_empty());
    let events = emitter.snapshot();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::SessionStart);
}

#[test]
fn session_new_with_required_cxdb_failure_returns_error() {
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let mut config = SessionConfig::default();
    config.cxdb_persistence = CxdbPersistenceMode::Required;
    let store = Arc::new(RecordingPersistence::with_failures(true, false));

    let error = Session::new_with_persistence(profile, env, client, config, Some(store))
        .err()
        .expect("required cxdb create failure should fail constructor");
    assert!(error.to_string().contains("cxdb persistence failed"));
}

#[test]
fn session_new_with_off_cxdb_failure_succeeds() {
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let config = SessionConfig::default();
    let store = Arc::new(RecordingPersistence::with_failures(true, false));

    let session = Session::new_with_persistence(profile, env, client, config, Some(store))
        .expect("off mode should keep constructor successful");
    assert_eq!(session.state(), &SessionState::Idle);
}

#[tokio::test(flavor = "current_thread")]
async fn submit_with_cxdb_persistence_persists_turns_and_tool_events() {
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: tool_registry_with_echo(),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let (client, _) = build_test_client(vec![
        tool_call_response(
            "resp-1",
            "call-1",
            "echo_tool",
            serde_json::json!({"value":"hello"}),
        ),
        text_response("resp-2", "done"),
    ]);
    let mut config = SessionConfig::default();
    config.cxdb_persistence = CxdbPersistenceMode::Required;
    let store = Arc::new(RecordingPersistence::default());
    let mut session =
        Session::new_with_persistence(profile, env, client, config, Some(store.clone()))
            .expect("session should initialize");

    session
        .submit("hi")
        .await
        .expect("submit should succeed with cxdb persistence");
    session.close().expect("close should succeed");

    let appended = store.appended();
    assert!(!appended.is_empty());
    let type_ids: Vec<&str> = appended
        .iter()
        .map(|request| request.type_id.as_str())
        .collect();
    assert!(type_ids.contains(&"forge.agent.user_turn"));
    assert!(type_ids.contains(&"forge.agent.assistant_turn"));
    assert!(type_ids.contains(&"forge.agent.tool_results_turn"));
    assert!(type_ids.contains(&"forge.agent.session_lifecycle"));
    assert!(type_ids.contains(&"forge.agent.tool_call_lifecycle"));

    let session_kinds: Vec<String> = appended
        .iter()
        .filter(|request| request.type_id == "forge.agent.session_lifecycle")
        .filter_map(|request| {
            decode_typed_record::<SessionLifecycleRecord>(&request.payload)
                .ok()
                .map(|record| record.kind)
        })
        .collect();
    assert!(session_kinds.iter().any(|kind| kind == "started"));
    assert!(session_kinds.iter().any(|kind| kind == "ended"));

    let tool_kinds: Vec<String> = appended
        .iter()
        .filter(|request| request.type_id == "forge.agent.tool_call_lifecycle")
        .filter_map(|request| {
            decode_typed_record::<ToolCallLifecycleRecord>(&request.payload)
                .ok()
                .map(|record| record.kind)
        })
        .collect();
    assert!(tool_kinds.iter().any(|kind| kind == "started"));
    assert!(tool_kinds.iter().any(|kind| kind == "ended"));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_with_fs_snapshot_policy_adds_fs_lineage_to_persisted_payloads() {
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let (client, _) = build_test_client(vec![text_response("resp-1", "done")]);
    let mut config = SessionConfig::default();
    config.cxdb_persistence = CxdbPersistenceMode::Required;
    config.fs_snapshot_policy = Some(CxdbFsSnapshotPolicy::default());
    let store = Arc::new(RecordingPersistence::default());
    let mut session =
        Session::new_with_persistence(profile, env, client, config, Some(store.clone()))
            .expect("session should initialize");

    session
        .submit("hi")
        .await
        .expect("submit should succeed with fs snapshot policy");

    let appended = store.appended();
    assert!(!appended.is_empty());
    assert!(
        appended
            .iter()
            .all(|request| request.fs_root_hash.is_some())
    );

    let first = &appended[0];
    if first.type_id == "forge.agent.session_lifecycle" {
        let record: SessionLifecycleRecord =
            decode_typed_record(&first.payload).expect("first payload should decode");
        assert!(record.fs_root_hash.is_some());
        assert!(record.snapshot_policy_id.is_some());
        assert!(record.snapshot_stats.is_some());
    } else {
        let record: AgentTurnRecord =
            decode_typed_record(&first.payload).expect("first payload should decode");
        assert!(record.fs_root_hash.is_some());
        assert!(record.snapshot_policy_id.is_some());
        assert!(record.snapshot_stats.is_some());
    }
}

#[test]
fn session_rejects_steer_when_closed() {
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");
    session.close().expect("close should succeed");

    let err = session.steer("halt").expect_err("steer should fail");
    assert!(matches!(err, AgentError::Session(SessionError::Closed)));
}

#[test]
fn session_state_enforces_spec_transitions() {
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    session
        .transition_to(SessionState::Processing)
        .expect("idle -> processing should work");
    session
        .transition_to(SessionState::AwaitingInput)
        .expect("processing -> awaiting_input should work");
    session
        .transition_to(SessionState::Processing)
        .expect("awaiting_input -> processing should work");
    session
        .transition_to(SessionState::Idle)
        .expect("processing -> idle should work");

    let err = session
        .transition_to(SessionState::AwaitingInput)
        .expect_err("idle -> awaiting_input should fail");
    assert!(matches!(
        err,
        AgentError::Session(SessionError::InvalidStateTransition { .. })
    ));
}

#[test]
fn closing_session_emits_session_end_once_with_final_state() {
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let mut session = Session::new_with_emitter(
        profile,
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("session should initialize");

    session.close().expect("close should succeed");
    session.close().expect("second close should be a no-op");

    let events = emitter.snapshot();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, EventKind::SessionStart);
    assert_eq!(events[1].kind, EventKind::SessionEnd);
    assert_eq!(events[1].data.get_str("final_state"), Some("CLOSED"));
}

#[test]
fn session_exposes_async_event_subscription() {
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let client = Arc::new(Client::default());
    let session =
        Session::new_with_emitter(profile, env, client, SessionConfig::default(), emitter)
            .expect("session should initialize");

    let mut stream = session.subscribe_events();
    session
        .emit(
            EventKind::UserInput,
            EventData::from_serializable(serde_json::json!({ "content": "hi" }))
                .expect("valid object payload"),
        )
        .expect("emit should succeed");

    let first = block_on(stream.next()).expect("session start should arrive");
    assert_eq!(first.kind, EventKind::SessionStart);
    let second = block_on(stream.next()).expect("user input should arrive");
    assert_eq!(second.kind, EventKind::UserInput);
}

#[tokio::test(flavor = "current_thread")]
async fn submit_natural_completion_without_tool_calls_returns_to_idle() {
    let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    session
        .submit("hello")
        .await
        .expect("submit should succeed");

    assert_eq!(session.state(), &SessionState::Idle);
    assert_eq!(session.history().len(), 2);
    assert!(matches!(session.history()[0], Turn::User(_)));
    assert!(matches!(session.history()[1], Turn::Assistant(_)));
    assert_eq!(requests.lock().expect("requests mutex").len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn submit_transitions_to_awaiting_input_for_question_then_back_to_idle_on_answer() {
    let (client, requests) = build_test_client(vec![
        text_response("resp-1", "Which file should I edit next?"),
        text_response("resp-2", "Done."),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    session
        .submit("start")
        .await
        .expect("first submit should succeed");
    assert_eq!(session.state(), &SessionState::AwaitingInput);

    session
        .submit("Edit src/main.rs")
        .await
        .expect("answer submit should succeed");
    assert_eq!(session.state(), &SessionState::Idle);
    assert_eq!(requests.lock().expect("requests mutex").len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn submit_enforces_per_input_round_limit_and_emits_turn_limit_event() {
    let (client, requests) = build_test_client(vec![
        tool_call_response(
            "resp-1",
            "call-1",
            "echo_tool",
            serde_json::json!({ "value": "first" }),
        ),
        text_response("resp-2", "should_not_be_called"),
    ]);
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: tool_registry_with_echo(),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut config = SessionConfig::default();
    config.max_tool_rounds_per_input = 1;
    let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
        .expect("new session");

    session
        .submit("run tool")
        .await
        .expect("submit should succeed");

    let events = emitter.snapshot();
    assert!(
        events
            .iter()
            .any(|event| event.kind == EventKind::TurnLimit)
    );
    assert_eq!(requests.lock().expect("requests mutex").len(), 1);
    assert_eq!(session.state(), &SessionState::Idle);
    assert_eq!(session.history().len(), 3);
    assert!(matches!(session.history()[2], Turn::ToolResults(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_multiple_times_keeps_history_consistent() {
    let (client, requests) = build_test_client(vec![
        text_response("resp-1", "first"),
        text_response("resp-2", "second"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    session.submit("one").await.expect("first submit");
    session.submit("two").await.expect("second submit");

    assert_eq!(session.state(), &SessionState::Idle);
    assert_eq!(session.history().len(), 4);
    assert!(matches!(session.history()[0], Turn::User(_)));
    assert!(matches!(session.history()[1], Turn::Assistant(_)));
    assert!(matches!(session.history()[2], Turn::User(_)));
    assert!(matches!(session.history()[3], Turn::Assistant(_)));
    assert_eq!(requests.lock().expect("requests mutex").len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn steering_messages_are_injected_into_history_and_next_request() {
    let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");
    session
        .steer("Use concise output")
        .expect("steer should queue");

    session
        .submit("hello")
        .await
        .expect("submit should succeed");

    assert!(matches!(session.history()[1], Turn::Steering(_)));
    let requests = requests.lock().expect("requests mutex");
    let first_request = &requests[0];
    assert!(
        first_request
            .messages
            .iter()
            .any(|message| message.role == Role::User && message.text() == "Use concise output")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn follow_up_queue_triggers_new_processing_cycle_after_completion() {
    let (client, requests) = build_test_client(vec![
        text_response("resp-1", "first"),
        text_response("resp-2", "second"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");
    session
        .follow_up("second input")
        .expect("follow-up should queue");

    session
        .submit("first input")
        .await
        .expect("submit should succeed");

    assert_eq!(session.history().len(), 4);
    assert!(matches!(&session.history()[0], Turn::User(turn) if turn.content == "first input"));
    assert!(matches!(&session.history()[2], Turn::User(turn) if turn.content == "second input"));
    assert_eq!(requests.lock().expect("requests mutex").len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn loop_detection_injects_warning_steering_turn_and_event() {
    let (client, requests) = build_test_client(vec![
        tool_call_response(
            "resp-1",
            "call-1",
            "tool_a",
            serde_json::json!({ "value": "a" }),
        ),
        tool_call_response(
            "resp-2",
            "call-2",
            "tool_b",
            serde_json::json!({ "value": "b" }),
        ),
        tool_call_response(
            "resp-3",
            "call-3",
            "tool_a",
            serde_json::json!({ "value": "a" }),
        ),
        tool_call_response(
            "resp-4",
            "call-4",
            "tool_b",
            serde_json::json!({ "value": "b" }),
        ),
        text_response("resp-5", "done"),
    ]);
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: tool_registry_with_named_echoes(&["tool_a", "tool_b"]),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut config = SessionConfig::default();
    config.loop_detection_window = 4;
    let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
        .expect("new session");

    session
        .submit("start")
        .await
        .expect("submit should succeed");

    assert!(session.history().iter().any(|turn| matches!(
        turn,
        Turn::Steering(turn) if turn.content.contains("Loop detected")
    )));
    assert!(
        emitter
            .snapshot()
            .iter()
            .any(|event| event.kind == EventKind::LoopDetection)
    );

    let requests = requests.lock().expect("requests mutex");
    assert!(
        requests[4].messages.iter().any(|message| {
            message.role == Role::User && message.text().contains("Loop detected")
        })
    );
}

#[tokio::test(flavor = "current_thread")]
async fn reasoning_effort_updates_apply_to_next_llm_call() {
    let (client, requests) = build_test_client(vec![
        text_response("resp-1", "first"),
        text_response("resp-2", "second"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    session
        .set_reasoning_effort(Some("low".to_string()))
        .expect("low should be valid");
    session.submit("one").await.expect("first submit");
    session
        .set_reasoning_effort(Some("high".to_string()))
        .expect("high should be valid");
    session.submit("two").await.expect("second submit");

    let requests = requests.lock().expect("requests mutex");
    assert_eq!(requests[0].reasoning_effort.as_deref(), Some("low"));
    assert_eq!(requests[1].reasoning_effort.as_deref(), Some("high"));

    let err = session
        .set_reasoning_effort(Some("ultra".to_string()))
        .expect_err("invalid value should be rejected");
    assert!(matches!(
        err,
        AgentError::Session(SessionError::InvalidConfiguration(_))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_emits_context_usage_warning_event_when_history_exceeds_threshold() {
    let (client, _requests) = build_test_client(vec![text_response("resp-1", "done")]);
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities {
            context_window_size: 10,
            ..ProviderCapabilities::default()
        },
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session = Session::new_with_emitter(
        profile,
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("new session");

    session
        .submit("x".repeat(64))
        .await
        .expect("submit should succeed");

    let events = emitter.snapshot();
    let warning = events
        .iter()
        .find(|event| {
            event.kind == EventKind::Warning
                && event.data.get_str("category") == Some("context_usage")
        })
        .expect("context usage warning event should be emitted");
    assert_eq!(warning.data.get_str("severity"), Some("warning"));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_does_not_emit_context_usage_warning_when_usage_is_below_threshold() {
    let (client, _requests) = build_test_client(vec![text_response("resp-1", "done")]);
    let emitter = Arc::new(BufferedEventEmitter::default());
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities {
            context_window_size: 8_000,
            ..ProviderCapabilities::default()
        },
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session = Session::new_with_emitter(
        profile,
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("new session");

    session.submit("hi").await.expect("submit should succeed");

    let events = emitter.snapshot();
    assert!(!events.iter().any(|event| {
        event.kind == EventKind::Warning && event.data.get_str("category") == Some("context_usage")
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn abort_handle_cancels_inflight_llm_call_and_closes_session() {
    let (client, _requests) = build_test_client_with_delay(
        vec![text_response("resp-1", "should not complete normally")],
        2_000,
    );
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: tool_registry_with_echo(),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let emitter = Arc::new(BufferedEventEmitter::default());
    let mut session = Session::new_with_emitter(
        profile,
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("new session");

    let abort_handle = session.abort_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        abort_handle.request_abort();
    });

    let started = std::time::Instant::now();
    session
        .submit("trigger abort")
        .await
        .expect("submit should complete cleanly on abort");

    assert_eq!(session.state(), &SessionState::Closed);
    assert!(started.elapsed() < std::time::Duration::from_millis(800));
    assert!(
        emitter
            .snapshot()
            .iter()
            .any(|event| event.kind == EventKind::SessionEnd),
        "expected SESSION_END after abort"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn abort_handle_terminates_running_shell_command() {
    #[cfg(windows)]
    let command = "ping -n 6 127.0.0.1 > NUL";
    #[cfg(not(windows))]
    let command = "sleep 5";

    let (client, _requests) = build_test_client(vec![tool_call_response(
        "resp-1",
        "call-shell",
        "shell",
        serde_json::json!({ "command": command }),
    )]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(build_openai_tool_registry()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env_dir = tempdir().expect("temp dir should be created");
    let env = Arc::new(LocalExecutionEnvironment::new(env_dir.path()));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let abort_handle = session.abort_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        abort_handle.request_abort();
    });

    let started = std::time::Instant::now();
    session
        .submit("run long command")
        .await
        .expect("submit should complete after abort");

    assert_eq!(session.state(), &SessionState::Closed);
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

#[test]
fn discover_project_documents_respects_provider_filter_and_precedence() {
    let tmp = tempdir().expect("temp dir should be created");
    let root = tmp.path();
    let nested = root.join("apps/service");
    fs::create_dir_all(&nested).expect("nested dir should be created");
    fs::create_dir_all(root.join(".git")).expect(".git marker dir should be created");

    write_test_file(&root.join("AGENTS.md"), "root agents");
    write_test_file(&root.join("CLAUDE.md"), "root claude");
    write_test_file(&root.join(".codex/instructions.md"), "root codex");
    write_test_file(&root.join("apps/AGENTS.md"), "apps agents");
    write_test_file(&root.join("apps/CLAUDE.md"), "apps claude");
    write_test_file(&root.join("apps/service/AGENTS.md"), "service agents");

    let profile = StaticProviderProfile {
        id: "anthropic".to_string(),
        model: "claude".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    };

    let docs = discover_project_documents(&nested, &profile);
    let paths: Vec<String> = docs.iter().map(|doc| doc.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            "AGENTS.md".to_string(),
            "CLAUDE.md".to_string(),
            "apps/AGENTS.md".to_string(),
            "apps/CLAUDE.md".to_string(),
            "apps/service/AGENTS.md".to_string()
        ]
    );
    assert!(docs.iter().all(|doc| doc.path != ".codex/instructions.md"));
}

#[test]
fn discover_project_documents_truncates_to_32kb_with_marker() {
    let tmp = tempdir().expect("temp dir should be created");
    let root = tmp.path();
    let nested = root.join("workspace");
    fs::create_dir_all(&nested).expect("nested dir should be created");
    fs::create_dir_all(root.join(".git")).expect(".git marker dir should be created");

    let oversized = "A".repeat(40 * 1024);
    write_test_file(&root.join("AGENTS.md"), &oversized);

    let profile = StaticProviderProfile {
        id: "openai".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    };

    let docs = discover_project_documents(&nested, &profile);
    assert_eq!(docs.len(), 1);
    assert!(docs[0].content.contains(PROJECT_DOC_TRUNCATION_MARKER));
    assert!(docs[0].content.len() <= (32 * 1024) + PROJECT_DOC_TRUNCATION_MARKER.len() + 1);
}

fn build_tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
        raw_arguments: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn subagent_tools_spawn_and_wait_flow_returns_deterministic_result() {
    let (client, _) = build_test_client(vec![text_response("child-resp-1", "child complete")]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let spawn = session
        .execute_subagent_tool_call(build_tool_call(
            "call-1",
            "spawn_agent",
            serde_json::json!({ "task": "do child task" }),
        ))
        .await
        .expect("spawn should execute");
    assert!(!spawn.is_error);
    let spawn_payload: Value = serde_json::from_str(
        spawn
            .content
            .as_str()
            .expect("spawn payload should be string JSON"),
    )
    .expect("spawn payload should parse");
    let agent_id = spawn_payload
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("agent_id must exist");
    assert_eq!(
        spawn_payload.get("status").and_then(Value::as_str),
        Some("running")
    );

    let wait = session
        .execute_subagent_tool_call(build_tool_call(
            "call-2",
            "wait",
            serde_json::json!({ "agent_id": agent_id }),
        ))
        .await
        .expect("wait should execute");
    assert!(!wait.is_error);
    let wait_payload: Value = serde_json::from_str(
        wait.content
            .as_str()
            .expect("wait payload should be string JSON"),
    )
    .expect("wait payload should parse");
    assert_eq!(
        wait_payload.get("agent_id").and_then(Value::as_str),
        Some(agent_id)
    );
    assert_eq!(
        wait_payload.get("status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        wait_payload.get("success").and_then(Value::as_bool),
        Some(true)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_agent_honors_model_override_for_child_requests() {
    let (client, requests) = build_test_client(vec![text_response("child-resp-1", "done")]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(build_openai_tool_registry()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let spawn = session
        .execute_subagent_tool_call(build_tool_call(
            "call-1",
            "spawn_agent",
            serde_json::json!({ "task": "do child task", "model": "override-model" }),
        ))
        .await
        .expect("spawn should execute");
    assert!(!spawn.is_error);
    let spawn_payload: Value = serde_json::from_str(
        spawn
            .content
            .as_str()
            .expect("spawn payload should be string JSON"),
    )
    .expect("spawn payload should parse");
    let agent_id = spawn_payload
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("agent_id must exist");

    let wait = session
        .execute_subagent_tool_call(build_tool_call(
            "call-2",
            "wait",
            serde_json::json!({ "agent_id": agent_id }),
        ))
        .await
        .expect("wait should execute");
    assert!(!wait.is_error);

    let seen_requests = requests.lock().expect("requests mutex").clone();
    assert_eq!(seen_requests.len(), 1);
    assert_eq!(seen_requests[0].model, "override-model");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_agent_honors_working_dir_scope_for_child_tools() {
    let temp = tempdir().expect("temp dir should exist");
    let scoped_dir = temp.path().join("scoped");
    fs::create_dir_all(&scoped_dir).expect("scoped dir should exist");
    fs::write(scoped_dir.join("only.txt"), "scoped-data\n").expect("seed file should write");

    let (client, _requests) = build_test_client(vec![
        tool_call_response(
            "child-resp-1",
            "call-read",
            "read_file",
            serde_json::json!({ "file_path": "only.txt", "offset": 1, "limit": 10 }),
        ),
        text_response("child-resp-2", "done"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(build_openai_tool_registry()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(temp.path()));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let spawn = session
        .execute_subagent_tool_call(build_tool_call(
            "call-1",
            "spawn_agent",
            serde_json::json!({ "task": "read file", "working_dir": "scoped" }),
        ))
        .await
        .expect("spawn should execute");
    assert!(!spawn.is_error);
    let spawn_payload: Value = serde_json::from_str(
        spawn
            .content
            .as_str()
            .expect("spawn payload should be string JSON"),
    )
    .expect("spawn payload should parse");
    let agent_id = spawn_payload
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("agent_id must exist");

    let wait = session
        .execute_subagent_tool_call(build_tool_call(
            "call-2",
            "wait",
            serde_json::json!({ "agent_id": agent_id }),
        ))
        .await
        .expect("wait should execute");
    assert!(!wait.is_error);

    let record = session
        .subagent_records
        .get(agent_id)
        .expect("subagent record should exist");
    let child = record
        .session
        .as_ref()
        .expect("child session should be available");
    let read_result = child.history().iter().find_map(|turn| {
        if let Turn::ToolResults(results) = turn {
            results
                .results
                .iter()
                .find(|result| result.tool_call_id == "call-read")
                .cloned()
        } else {
            None
        }
    });
    let read_result = read_result.expect("read_file result should be present");
    assert!(!read_result.is_error);
    assert!(
        read_result
            .content
            .as_str()
            .unwrap_or_default()
            .contains("scoped-data")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_agent_rejects_when_depth_limit_reached() {
    let (client, _) = build_test_client(vec![]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut config = SessionConfig::default();
    config.max_subagent_depth = 0;
    let mut session = Session::new(profile, env, client, config).expect("new session");

    let result = session
        .execute_subagent_tool_call(build_tool_call(
            "call-1",
            "spawn_agent",
            serde_json::json!({ "task": "blocked" }),
        ))
        .await
        .expect("tool execution should not panic");

    assert!(result.is_error);
    assert!(
        result
            .content
            .as_str()
            .unwrap_or_default()
            .contains("max_subagent_depth")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn close_closes_all_subagents_and_updates_status() {
    let (client, _) = build_test_client(vec![text_response("child-resp-1", "done")]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "gpt-5.2-codex".to_string(),
        base_system_prompt: "system".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let spawn = session
        .execute_subagent_tool_call(build_tool_call(
            "call-1",
            "spawn_agent",
            serde_json::json!({ "task": "run child" }),
        ))
        .await
        .expect("spawn should execute");
    let spawn_payload: Value =
        serde_json::from_str(spawn.content.as_str().expect("spawn content")).expect("json");
    let agent_id = spawn_payload
        .get("agent_id")
        .and_then(Value::as_str)
        .expect("agent id");
    assert!(session.subagents.contains_key(agent_id));

    session.close().expect("close should succeed");
    assert_eq!(session.state(), &SessionState::Closed);
    assert!(matches!(
        session.subagents.get(agent_id).map(|h| &h.status),
        Some(SubAgentStatus::Failed)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn submit_with_options_overrides_provider_model_and_reasoning() {
    let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
    let base_profile = Arc::new(StaticProviderProfile {
        id: "base".to_string(),
        model: "base-model".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let alt_profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "alt-model".to_string(),
        base_system_prompt: "alt".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(base_profile, env, client, SessionConfig::default()).expect("new session");
    session.register_provider_profile(alt_profile);

    let mut metadata = HashMap::new();
    metadata.insert("node".to_string(), "plan".to_string());
    session
        .submit_with_options(
            "hello",
            SubmitOptions {
                provider: Some("test".to_string()),
                model: Some("override-model".to_string()),
                reasoning_effort: Some("low".to_string()),
                system_prompt_override: Some("node override".to_string()),
                provider_options: Some(serde_json::json!({ "x": 1 })),
                metadata: Some(metadata.clone()),
            },
        )
        .await
        .expect("submit should succeed");

    let seen = requests.lock().expect("requests mutex");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].provider.as_deref(), Some("test"));
    assert_eq!(seen[0].model, "override-model");
    assert_eq!(seen[0].reasoning_effort.as_deref(), Some("low"));
    assert_eq!(seen[0].metadata.as_ref(), Some(&metadata));
    assert_eq!(
        seen[0].provider_options,
        Some(serde_json::json!({ "x": 1 }))
    );
    assert!(
        seen[0]
            .messages
            .first()
            .expect("system message")
            .content
            .iter()
            .any(|part| part
                .text
                .as_deref()
                .is_some_and(|text| text.contains("node override")))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn submit_with_result_returns_tool_ids_usage_and_thread_key() {
    let (client, _requests) = build_test_client(vec![
        tool_call_response(
            "resp-1",
            "call-read",
            "read_file",
            serde_json::json!({ "file_path": "Cargo.toml" }),
        ),
        text_response("resp-2", "done"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "test-model".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(build_openai_tool_registry()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut config = SessionConfig::default();
    config.thread_key = Some("thread-main".to_string());
    let mut session = Session::new(profile, env, client, config).expect("new session");

    let result = session
        .submit_with_result("run tool", SubmitOptions::default())
        .await
        .expect("submit should succeed");
    assert_eq!(result.final_state, SessionState::Idle);
    assert_eq!(result.assistant_text, "done");
    assert_eq!(result.tool_call_count, 1);
    assert_eq!(result.tool_call_ids, vec!["call-read".to_string()]);
    assert_eq!(result.tool_error_count, 0);
    assert_eq!(result.thread_key.as_deref(), Some("thread-main"));
    let usage = result.usage.expect("usage should exist");
    assert!(usage.total_tokens > 0);
}

#[tokio::test(flavor = "current_thread")]
async fn checkpoint_round_trip_restores_history_and_queues() {
    let (client, _requests) = build_test_client(vec![
        text_response("resp-1", "first"),
        text_response("resp-2", "second"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "test-model".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let emitter = Arc::new(BufferedEventEmitter::default());
    let mut session = Session::new_with_emitter(
        profile.clone(),
        env.clone(),
        client.clone(),
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("new session");
    session.submit("first input").await.expect("first submit");
    session.steer("queued steering").expect("steer queued");
    session
        .follow_up("queued followup")
        .expect("followup queued");
    session.set_thread_key(Some("thread-restore".to_string()));

    let checkpoint = session.checkpoint().expect("checkpoint should succeed");
    let mut restored = Session::from_checkpoint(checkpoint.clone(), profile, env, client, emitter)
        .expect("restore should succeed");
    assert_eq!(restored.id(), checkpoint.session_id);
    assert_eq!(restored.state(), &checkpoint.state);
    assert_eq!(restored.history(), checkpoint.history.as_slice());
    assert_eq!(
        restored.pop_steering_message().as_deref(),
        Some("queued steering")
    );
    assert_eq!(
        restored.pop_followup_message().as_deref(),
        Some("queued followup")
    );
    assert_eq!(restored.thread_key(), Some("thread-restore"));
    assert_eq!(checkpoint.thread_key.as_deref(), Some("thread-restore"));

    restored
        .submit("second input")
        .await
        .expect("second submit");
    assert!(restored.history().iter().any(|turn| {
        matches!(turn, Turn::Assistant(assistant) if assistant.content == "second")
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn checkpoint_fails_when_subagent_task_is_running() {
    let (client, _requests) = build_test_client(vec![]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "test-model".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(ToolRegistry::default()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let active_task = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        panic!("task should be aborted by test");
    });
    session.subagent_records.insert(
        "agent-1".to_string(),
        SubAgentRecord {
            session: None,
            active_task: Some(active_task),
            result: None,
        },
    );

    let error = session.checkpoint().expect_err("checkpoint should fail");
    assert!(matches!(
        error,
        AgentError::Session(SessionError::CheckpointUnsupported(_))
    ));
    if let Some(record) = session.subagent_records.get_mut("agent-1") {
        if let Some(task) = record.active_task.take() {
            task.abort();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tool_hook_runs_for_regular_and_subagent_tools() {
    let (client, _requests) = build_test_client(vec![
        tool_call_response(
            "resp-1",
            "call-read",
            "read_file",
            serde_json::json!({"file_path":"Cargo.toml"}),
        ),
        text_response("resp-2", "done"),
    ]);
    let profile = Arc::new(StaticProviderProfile {
        id: "test".to_string(),
        model: "test-model".to_string(),
        base_system_prompt: "base".to_string(),
        tool_registry: Arc::new(build_openai_tool_registry()),
        provider_options: None,
        capabilities: ProviderCapabilities::default(),
    });
    let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
    let mut session =
        Session::new(profile, env, client, SessionConfig::default()).expect("new session");

    let hook = Arc::new(RecordingHook {
        pre_calls: Mutex::new(Vec::new()),
        post_calls: Mutex::new(Vec::new()),
        skip_tool_name: Some("spawn_agent".to_string()),
    });
    session.set_tool_call_hook(Some(hook.clone()));
    session
        .submit("run read")
        .await
        .expect("submit should work");
    let skipped = session
        .execute_subagent_tool_call(build_tool_call(
            "call-sub",
            "spawn_agent",
            serde_json::json!({"task":"should skip"}),
        ))
        .await
        .expect("subagent call should return");
    assert!(skipped.is_error);
    assert!(
        skipped
            .content
            .as_str()
            .unwrap_or_default()
            .contains("skipped spawn_agent")
    );
    assert!(session.subagents().is_empty());

    let pre_calls = hook.pre_calls.lock().expect("pre lock").clone();
    let post_calls = hook.post_calls.lock().expect("post lock").clone();
    assert!(pre_calls.iter().any(|name| name == "read_file"));
    assert!(pre_calls.iter().any(|name| name == "spawn_agent"));
    assert!(post_calls.iter().any(|name| name == "read_file"));
    assert!(!post_calls.iter().any(|name| name == "spawn_agent"));
}
