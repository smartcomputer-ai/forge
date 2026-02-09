use async_trait::async_trait;
use forge_agent::{
    AnthropicProviderProfile, AssistantTurn, LocalExecutionEnvironment, OpenAiProviderProfile,
    ProviderProfile, Session, SessionConfig, ToolResultTurn, Turn,
};
use forge_agent::{ExecutionEnvironment, GeminiProviderProfile, ToolResultsTurn};
use forge_llm::{
    Client, ConfigurationError, ContentPart, FinishReason, Message, ProviderAdapter, Request,
    Response, SDKError, StreamEventStream, ToolCallData, Usage,
};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Clone)]
struct SequenceAdapter {
    name: String,
    responses: Arc<Mutex<VecDeque<Response>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

#[async_trait]
impl ProviderAdapter for SequenceAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, request: Request) -> Result<Response, SDKError> {
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

#[derive(Clone, Copy)]
enum FixtureKind {
    OpenAi,
    Anthropic,
    Gemini,
}

impl FixtureKind {
    fn id(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    fn model(&self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.2-codex",
            Self::Anthropic => "claude-sonnet-4.5",
            Self::Gemini => "gemini-2.5-pro",
        }
    }

    fn profile(&self) -> Arc<dyn ProviderProfile> {
        match self {
            Self::OpenAi => Arc::new(OpenAiProviderProfile::with_default_tools(self.model())),
            Self::Anthropic => Arc::new(AnthropicProviderProfile::with_default_tools(self.model())),
            Self::Gemini => Arc::new(GeminiProviderProfile::with_default_tools(self.model())),
        }
    }

    fn edit_tool_name(&self) -> &'static str {
        match self {
            Self::OpenAi => "apply_patch",
            Self::Anthropic | Self::Gemini => "edit_file",
        }
    }
}

fn all_fixtures() -> [FixtureKind; 3] {
    [FixtureKind::OpenAi, FixtureKind::Anthropic, FixtureKind::Gemini]
}

fn usage() -> Usage {
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

fn text_response(provider: &str, model: &str, id: &str, text: &str) -> Response {
    Response {
        id: id.to_string(),
        model: model.to_string(),
        provider: provider.to_string(),
        message: Message::assistant(text),
        finish_reason: FinishReason {
            reason: "stop".to_string(),
            raw: None,
        },
        usage: usage(),
        raw: None,
        warnings: Vec::new(),
        rate_limit: None,
    }
}

fn tool_call_response(
    provider: &str,
    model: &str,
    id: &str,
    calls: Vec<(&str, &str, serde_json::Value)>,
) -> Response {
    let parts = calls
        .into_iter()
        .map(|(call_id, name, arguments)| {
            ContentPart::tool_call(ToolCallData {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments,
                r#type: "function".to_string(),
            })
        })
        .collect();

    Response {
        id: id.to_string(),
        model: model.to_string(),
        provider: provider.to_string(),
        message: Message {
            role: forge_llm::Role::Assistant,
            content: parts,
            name: None,
            tool_call_id: None,
        },
        finish_reason: FinishReason {
            reason: "tool_calls".to_string(),
            raw: None,
        },
        usage: usage(),
        raw: None,
        warnings: Vec::new(),
        rate_limit: None,
    }
}

fn client_with_adapter(
    provider_name: &str,
) -> (
    Arc<Client>,
    Arc<Mutex<VecDeque<Response>>>,
    Arc<Mutex<Vec<Request>>>,
) {
    let responses = Arc::new(Mutex::new(VecDeque::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let adapter = Arc::new(SequenceAdapter {
        name: provider_name.to_string(),
        responses: responses.clone(),
        requests: requests.clone(),
    });

    let mut client = Client::default();
    client
        .register_provider(adapter)
        .expect("provider should register");

    (Arc::new(client), responses, requests)
}

fn enqueue(responses: &Arc<Mutex<VecDeque<Response>>>, response: Response) {
    responses
        .lock()
        .expect("responses mutex")
        .push_back(response);
}

fn tool_result_by_call_id<'a>(history: &'a [Turn], call_id: &str) -> Option<&'a ToolResultTurn> {
    history.iter().find_map(|turn| {
        if let Turn::ToolResults(ToolResultsTurn { results, .. }) = turn {
            return results.iter().find(|result| result.tool_call_id == call_id);
        }
        None
    })
}

fn last_assistant_text(history: &[Turn]) -> Option<String> {
    history.iter().rev().find_map(|turn| {
        if let Turn::Assistant(AssistantTurn { content, .. }) = turn {
            Some(content.clone())
        } else {
            None
        }
    })
}

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
        assert!(env.file_exists("note.txt").await.expect("exists should work"));

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
        assert!(glob
            .content
            .as_str()
            .unwrap_or_default()
            .contains("main.txt"));

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
        assert!(timeout_result
            .content
            .as_str()
            .unwrap_or_default()
            .contains("Command timed out"));
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
            text_response(fixture.id(), fixture.model(), "resp-child-1", "child finished"),
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
