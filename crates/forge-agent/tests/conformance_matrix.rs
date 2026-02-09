mod support;

use forge_agent::{ExecutionEnvironment, LocalExecutionEnvironment, Session, SessionConfig};
use serde_json::json;
use std::sync::Arc;
use support::{
    FixtureKind, all_fixtures, client_with_adapter, enqueue, last_assistant_text, text_response,
    tool_call_response, tool_result_by_call_id,
};
use tempfile::tempdir;

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_file_creation_read_edit_and_native_variant() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env.clone(), client, SessionConfig::default())
            .expect("session should initialize");

        // Step 1: create file.
        match fixture {
            FixtureKind::OpenAi => {
                enqueue(
                    &responses,
                    tool_call_response(
                        fixture.id(),
                        fixture.model(),
                        "resp-1",
                        vec![(
                            "call-create",
                            "apply_patch",
                            json!({
                                "patch": "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch"
                            }),
                        )],
                    ),
                );
            }
            FixtureKind::Anthropic | FixtureKind::Gemini => {
                enqueue(
                    &responses,
                    tool_call_response(
                        fixture.id(),
                        fixture.model(),
                        "resp-1",
                        vec![(
                            "call-create",
                            "write_file",
                            json!({ "file_path": "note.txt", "content": "hello\n" }),
                        )],
                    ),
                );
            }
        }
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "created"),
        );

        session
            .submit("create note")
            .await
            .expect("create submit should succeed");
        assert!(
            env.file_exists("note.txt")
                .await
                .expect("exists should work")
        );

        // Step 2: read then provider-native edit.
        let edit_tool = fixture.edit_tool_name();
        let edit_args = match fixture {
            FixtureKind::OpenAi => json!({
                "patch": "*** Begin Patch\n*** Update File: note.txt\n@@ update\n-hello\n+hello\n+goodbye\n*** End Patch"
            }),
            FixtureKind::Anthropic | FixtureKind::Gemini => json!({
                "file_path": "note.txt",
                "old_string": "hello",
                "new_string": "hello\\ngoodbye",
                "replace_all": false
            }),
        };

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-3",
                vec![
                    (
                        "call-read",
                        "read_file",
                        json!({ "file_path": "note.txt", "offset": 1, "limit": 50 }),
                    ),
                    ("call-edit", edit_tool, edit_args),
                ],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-4", "edited"),
        );

        session
            .submit("read and edit note")
            .await
            .expect("edit submit should succeed");

        let content = env
            .read_file("note.txt", None, None)
            .await
            .expect("read back should succeed");
        assert!(content.contains("hello"));
        assert!(content.contains("goodbye"));

        let edit_result = tool_result_by_call_id(session.history(), "call-edit")
            .expect("edit tool result should exist");
        assert!(!edit_result.is_error);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_shell_search_and_timeout_flows() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("src/main.txt", "alpha\nbeta\ngamma\n")
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
                vec![(
                    "call-shell-ok",
                    "shell",
                    json!({ "command": "echo conformance-ok" }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "shell ok"),
        );
        session
            .submit("run shell")
            .await
            .expect("shell submit should succeed");

        let shell_ok = tool_result_by_call_id(session.history(), "call-shell-ok")
            .expect("shell result should exist");
        let shell_ok_text = shell_ok.content.as_str().unwrap_or_default();
        assert!(shell_ok_text.contains("exit_code: 0"));

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-3",
                vec![
                    (
                        "call-grep",
                        "grep",
                        json!({ "pattern": "beta", "path": ".", "max_results": 10 }),
                    ),
                    (
                        "call-glob",
                        "glob",
                        json!({ "pattern": "**/*.txt", "path": "." }),
                    ),
                ],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-4", "search done"),
        );
        session
            .submit("search workspace")
            .await
            .expect("search submit should succeed");

        let grep = tool_result_by_call_id(session.history(), "call-grep")
            .expect("grep result should exist");
        assert!(grep.content.as_str().unwrap_or_default().contains("beta"));

        let glob = tool_result_by_call_id(session.history(), "call-glob")
            .expect("glob result should exist");
        assert!(
            glob.content
                .as_str()
                .unwrap_or_default()
                .contains("main.txt")
        );

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-5",
                vec![(
                    "call-timeout",
                    "shell",
                    json!({ "command": "echo start && sleep 1 && echo done", "timeout_ms": 20 }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-6", "timeout done"),
        );
        session
            .submit("run timeout command")
            .await
            .expect("timeout submit should succeed");

        let timeout_result = tool_result_by_call_id(session.history(), "call-timeout")
            .expect("timeout result should exist");
        assert!(
            timeout_result
                .content
                .as_str()
                .unwrap_or_default()
                .contains("Command timed out")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_parallel_tool_calls_and_subagent_spawn_wait() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("file.txt", "one\n")
            .await
            .expect("seed file should write");

        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env.clone(), client, SessionConfig::default())
            .expect("session should initialize");

        // Parallel-capable tool-call response.
        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-1",
                vec![
                    (
                        "call-parallel-a",
                        "read_file",
                        json!({ "file_path": "file.txt", "offset": 1, "limit": 10 }),
                    ),
                    (
                        "call-parallel-b",
                        "glob",
                        json!({ "pattern": "*.txt", "path": "." }),
                    ),
                ],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "parallel done"),
        );
        session
            .submit("parallel check")
            .await
            .expect("parallel submit should succeed");

        assert!(tool_result_by_call_id(session.history(), "call-parallel-a").is_some());
        assert!(tool_result_by_call_id(session.history(), "call-parallel-b").is_some());

        // Spawn flow: parent tool call -> child text response -> parent final text.
        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-3",
                vec![(
                    "call-spawn",
                    "spawn_agent",
                    json!({ "task": "write a one-line summary" }),
                )],
            ),
        );
        enqueue(
            &responses,
            text_response(
                fixture.id(),
                fixture.model(),
                "resp-child-1",
                "child finished",
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-4", "spawned"),
        );

        session
            .submit("spawn subagent")
            .await
            .expect("spawn submit should succeed");

        let agent_id = session
            .subagents()
            .keys()
            .next()
            .cloned()
            .expect("agent id should exist after spawn");

        enqueue(
            &responses,
            tool_call_response(
                fixture.id(),
                fixture.model(),
                "resp-5",
                vec![("call-wait", "wait", json!({ "agent_id": agent_id }))],
            ),
        );
        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-6", "waited"),
        );

        session
            .submit("wait for subagent")
            .await
            .expect("wait submit should succeed");

        let wait_result = tool_result_by_call_id(session.history(), "call-wait")
            .expect("wait result should exist");
        let wait_payload = wait_result.content.as_str().unwrap_or_default();
        assert!(wait_payload.contains("\"status\":\"completed\""));
        assert!(wait_payload.contains("\"success\":true"));

        let final_text = last_assistant_text(session.history()).unwrap_or_default();
        assert_eq!(final_text, "waited");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_profile_multi_file_edit_flow() {
    for fixture in all_fixtures() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("a.txt", "alpha\n")
            .await
            .expect("seed file should write");
        env.write_file("b.txt", "bravo\n")
            .await
            .expect("seed file should write");

        let (client, responses, _requests) = client_with_adapter(fixture.id());
        let profile = fixture.profile();
        let mut session = Session::new(profile, env.clone(), client, SessionConfig::default())
            .expect("session should initialize");

        match fixture {
            FixtureKind::OpenAi => {
                enqueue(
                    &responses,
                    tool_call_response(
                        fixture.id(),
                        fixture.model(),
                        "resp-1",
                        vec![(
                            "call-multi",
                            "apply_patch",
                            json!({
                                "patch": "*** Begin Patch\n*** Update File: a.txt\n@@\n-alpha\n+alpha\n+delta\n*** Update File: b.txt\n@@\n-bravo\n+bravo\n+echo\n*** End Patch"
                            }),
                        )],
                    ),
                );
            }
            FixtureKind::Anthropic | FixtureKind::Gemini => {
                enqueue(
                    &responses,
                    tool_call_response(
                        fixture.id(),
                        fixture.model(),
                        "resp-1",
                        vec![
                            (
                                "call-edit-a",
                                "edit_file",
                                json!({
                                    "file_path": "a.txt",
                                    "old_string": "alpha",
                                    "new_string": "alpha\\ndelta",
                                    "replace_all": false
                                }),
                            ),
                            (
                                "call-edit-b",
                                "edit_file",
                                json!({
                                    "file_path": "b.txt",
                                    "old_string": "bravo",
                                    "new_string": "bravo\\necho",
                                    "replace_all": false
                                }),
                            ),
                        ],
                    ),
                );
            }
        }

        enqueue(
            &responses,
            text_response(fixture.id(), fixture.model(), "resp-2", "multi edit done"),
        );

        session
            .submit("edit both files")
            .await
            .expect("multi-file submit should succeed");

        let a_content = env
            .read_file("a.txt", None, None)
            .await
            .expect("read should succeed");
        let b_content = env
            .read_file("b.txt", None, None)
            .await
            .expect("read should succeed");
        assert!(a_content.contains("delta"));
        assert!(b_content.contains("echo"));

        if matches!(fixture, FixtureKind::OpenAi) {
            let result = tool_result_by_call_id(session.history(), "call-multi")
                .expect("patch result should exist");
            assert!(!result.is_error);
        } else {
            assert!(tool_result_by_call_id(session.history(), "call-edit-a").is_some());
            assert!(tool_result_by_call_id(session.history(), "call-edit-b").is_some());
        }
    }
}
