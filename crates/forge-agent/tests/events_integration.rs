mod support;

use support::{
    client_with_adapter, enqueue, text_response, tool_call_response, tool_result_by_call_id,
};
use forge_agent::{
    BufferedEventEmitter, EventKind, ExecutionEnvironment, LocalExecutionEnvironment, Session,
    SessionConfig,
};
use std::sync::Arc;
use tempfile::tempdir;

fn event_index(events: &[forge_agent::SessionEvent], kind: EventKind) -> Option<usize> {
    events.iter().position(|event| event.kind == kind)
}

#[tokio::test(flavor = "current_thread")]
async fn event_sequence_smoke_emits_expected_lifecycle_and_tool_kinds() {
    let dir = tempdir().expect("temp dir should be created");
    let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
    env.write_file("demo.txt", "alpha\n")
        .await
        .expect("seed file should write");

    let (client, responses, _requests) = client_with_adapter("openai");
    let profile = forge_agent::OpenAiProviderProfile::with_default_tools("gpt-5.2-codex");
    let emitter = Arc::new(BufferedEventEmitter::default());
    let mut session = Session::new_with_emitter(
        Arc::new(profile),
        env,
        client,
        SessionConfig::default(),
        emitter.clone(),
    )
    .expect("session should initialize");

    enqueue(
        &responses,
        tool_call_response(
            "openai",
            "gpt-5.2-codex",
            "resp-1",
            vec![(
                "call-read",
                "read_file",
                serde_json::json!({ "file_path": "demo.txt", "offset": 1, "limit": 20 }),
            )],
        ),
    );
    enqueue(
        &responses,
        text_response("openai", "gpt-5.2-codex", "resp-2", "done"),
    );

    session.submit("run tool then finish").await.expect("submit");
    session.close().expect("close should succeed");

    let events = emitter.snapshot();
    assert!(event_index(&events, EventKind::SessionStart).is_some());
    assert!(event_index(&events, EventKind::UserInput).is_some());
    assert!(event_index(&events, EventKind::AssistantTextStart).is_some());
    assert!(event_index(&events, EventKind::AssistantTextEnd).is_some());
    assert!(event_index(&events, EventKind::ToolCallStart).is_some());
    assert!(event_index(&events, EventKind::ToolCallEnd).is_some());
    assert!(event_index(&events, EventKind::SessionEnd).is_some());

    let session_start_idx =
        event_index(&events, EventKind::SessionStart).expect("session start should exist");
    let session_end_idx =
        event_index(&events, EventKind::SessionEnd).expect("session end should exist");
    assert!(session_start_idx < session_end_idx);
}

#[tokio::test(flavor = "current_thread")]
async fn tool_call_end_event_carries_full_output_when_tool_result_is_truncated() {
    let dir = tempdir().expect("temp dir should be created");
    let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
    let long_text = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".repeat(8);
    env.write_file("long.txt", &long_text)
        .await
        .expect("seed file should write");

    let (client, responses, _requests) = client_with_adapter("openai");
    let profile = forge_agent::OpenAiProviderProfile::with_default_tools("gpt-5.2-codex");
    let emitter = Arc::new(BufferedEventEmitter::default());
    let mut config = SessionConfig::default();
    config.tool_output_limits.insert("read_file".to_string(), 60);
    let mut session =
        Session::new_with_emitter(Arc::new(profile), env, client, config, emitter.clone())
            .expect("session should initialize");

    enqueue(
        &responses,
        tool_call_response(
            "openai",
            "gpt-5.2-codex",
            "resp-1",
            vec![(
                "call-long",
                "read_file",
                serde_json::json!({ "file_path": "long.txt", "offset": 1, "limit": 500 }),
            )],
        ),
    );
    enqueue(
        &responses,
        text_response("openai", "gpt-5.2-codex", "resp-2", "done"),
    );

    session.submit("read long file").await.expect("submit");

    let tool_result = tool_result_by_call_id(session.history(), "call-long")
        .expect("tool result should exist")
        .content
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(tool_result.contains("[WARNING: Tool output was truncated."));

    let full_output_event = emitter
        .snapshot()
        .into_iter()
        .find(|event| {
            event.kind == EventKind::ToolCallEnd
                && event.data.get_str("call_id") == Some("call-long")
                && event.data.get_str("output").is_some()
        })
        .expect("tool call end output event should exist");
    let full_output = full_output_event
        .data
        .get_str("output")
        .expect("output should exist");

    assert!(full_output.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(full_output.len() > tool_result.len());
}
