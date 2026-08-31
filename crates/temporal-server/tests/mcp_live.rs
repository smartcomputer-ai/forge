//! End-to-end MCP acceptance coverage over a real Temporal worker and local
//! Stateless Streamable HTTP servers. Provider-hosted MCP has separate live
//! suites in `llm-runtime`; this suite owns Lightspeed-native execution and
//! the universe/session control plane.

mod support;

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use api::{
    AgentApiService, ApprovalDecisionInput, ApprovalDecisionKind, FeaturesConfig, InputItem,
    McpServerDeleteParams, McpServerInput, McpServerLink, McpServerPutParams, McpServerStatus,
    McpServerToolsDiscoverParams, McpServerToolsDiscoverResponse, RemoteMcpApprovalPolicy,
    RemoteMcpExecution, RemoteMcpExposure, RunApprovalsDecideParams, RunLimitsConfig,
    RunStartConfig, RunStartParams, RunStartSource, SessionConfig, SessionReadParams,
    SessionStartParams,
};
use api_projection::model_to_api;
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use engine::{
    ContextEntryInput, ContextEntryKind, ContextMessageRole, CoreAgentIoError, CoreAgentLlm,
    CoreAgentTools, LlmFinish, LlmGenerationFacts, LlmGenerationRequest, LlmGenerationResult,
    LlmGenerationStatus, ObservedToolCall, SessionId, ToolCallId, ToolName, storage::BlobStore,
};
use serde_json::{Value, json};
use support::live::{
    LIVE_TEST_LOCK, final_assistant_text, live_universe_id, live_workflow_handle,
    require_storage_live_env, run_with_live_worker, wait_for_terminal_run,
};
use temporal_server::{
    default_model_from_env,
    gateway::GatewayAgentApi,
    pg_store_from_env,
    worker::{ActivityState, FakeTools, WorkerActivities},
};
use temporalio_client::{Client, WorkflowTerminateOptions};
use tokio::sync::Mutex;

const LARGE_TOOL_COUNT: usize = 45;
const LARGE_PAGE_SIZE: usize = 15;

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn mcp_live_native_configuration_and_server_size_matrix() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let fixture = LiveMcpFixture::start().await?;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let ids = MatrixServerIds {
        small: format!("small_{suffix}"),
        large: format!("large_{suffix}"),
        selected: format!("selected_{suffix}"),
    };
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(MatrixScriptedLlm {
        blobs: blobs.clone(),
        ids: ids.clone(),
    }) as Arc<dyn CoreAgentLlm>;
    let tools = Arc::new(FakeTools::new(blobs)) as Arc<dyn CoreAgentTools>;
    let state = ActivityState::from_pg_store(store.clone(), llm, tools)
        .with_native_mcp_from_pg_store(store)?;
    let activities = WorkerActivities::for_universe(live_universe_id()?, state);
    let fixture_for_client = fixture.clone();
    let ids_for_client = ids.clone();

    let result = run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_matrix_client(
            client,
            task_queue,
            session_id,
            fixture_for_client,
            ids_for_client,
        )
    })
    .await;

    result?;
    fixture.assert_calls().await
}

#[derive(Clone)]
struct MatrixServerIds {
    small: String,
    large: String,
    selected: String,
}

#[derive(Clone)]
struct MatrixScriptedLlm {
    blobs: Arc<dyn BlobStore>,
    ids: MatrixServerIds,
}

#[async_trait]
impl CoreAgentLlm for MatrixScriptedLlm {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let latest = request
            .request
            .context
            .entries
            .iter()
            .rev()
            .find_map(|entry| match &entry.kind {
                ContextEntryKind::ToolResult { call_id, .. } => {
                    Some((call_id.as_str().to_owned(), entry.content_ref.clone()))
                }
                _ => None,
            });
        let Some((call_id, result_ref)) = latest else {
            return self
                .tool_call(
                    &request,
                    &format!("mcp_{}__echo", self.ids.small),
                    "matrix_inject_echo",
                    json!({"value": "hello"}),
                )
                .await;
        };
        let result = self.blobs.read_text(&result_ref).await.map_err(io_error)?;
        match call_id.as_str() {
            "matrix_inject_echo" => {
                require_contains(&result, "small-echo:hello", &call_id)?;
                self.tool_call(
                    &request,
                    &format!("mcp_{}__rich", self.ids.small),
                    "matrix_inject_rich",
                    json!({}),
                )
                .await
            }
            "matrix_inject_rich" => {
                require_contains(&result, "rich-text", &call_id)?;
                require_contains(
                    &result,
                    "[MCP image content stored as structured output]",
                    &call_id,
                )?;
                self.tool_call(
                    &request,
                    "mcp_find_tools",
                    "matrix_browse_1",
                    json!({"server": self.ids.large}),
                )
                .await
            }
            "matrix_browse_1" => {
                assert_find_page(&result, 20, Some(20), "catalog_tool_000")?;
                self.tool_call(
                    &request,
                    "mcp_find_tools",
                    "matrix_browse_2",
                    json!({"server": self.ids.large, "cursor": 20}),
                )
                .await
            }
            "matrix_browse_2" => {
                assert_find_page(&result, 20, Some(40), "catalog_tool_020")?;
                self.tool_call(
                    &request,
                    "mcp_find_tools",
                    "matrix_filtered",
                    json!({"server": self.ids.selected}),
                )
                .await
            }
            "matrix_filtered" => {
                assert_find_page(&result, 1, None, "catalog_tool_039")?;
                self.tool_call(
                    &request,
                    "mcp_find_tools",
                    "matrix_selected",
                    json!({"server": self.ids.selected, "query": "catlog tools 039"}),
                )
                .await
            }
            "matrix_selected" => {
                assert_find_page(&result, 1, None, "catalog_tool_039")?;
                require_contains(&result, "inputSchema", &call_id)?;
                self.tool_call(
                    &request,
                    "mcp_call",
                    "matrix_approved_call",
                    json!({
                        "server": self.ids.large,
                        "tool": "catalog_tool_039",
                        "arguments": {"value": "approved"}
                    }),
                )
                .await
            }
            "matrix_approved_call" => {
                require_contains(&result, "large-call:catalog_tool_039:approved", &call_id)?;
                self.final_result(&request, "native MCP matrix complete")
                    .await
            }
            other => Err(CoreAgentIoError::Failed {
                message: format!("unexpected MCP matrix tool result {other}: {result}"),
            }),
        }
    }
}

impl MatrixScriptedLlm {
    async fn tool_call(
        &self,
        request: &LlmGenerationRequest,
        tool: &str,
        call_id: &str,
        arguments: Value,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let arguments_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(&arguments).map_err(io_error)?)
            .await
            .map_err(io_error)?;
        let call_id = ToolCallId::new(call_id);
        let tool_name = ToolName::new(tool);
        Ok(LlmGenerationResult {
            run_id: request.run_id,
            turn_id: request.turn_id,
            status: LlmGenerationStatus::Succeeded,
            failure_ref: None,
            context_entries: vec![ContextEntryInput {
                kind: ContextEntryKind::ToolCall {
                    call_id: call_id.clone(),
                    name: tool_name.clone(),
                },
                content_ref: arguments_ref.clone(),
                media_type: Some("application/json".to_owned()),
                preview: None,
                provider_kind: Some("mcp-live-matrix".to_owned()),
                provider_item_id: Some(call_id.as_str().to_owned()),
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("mcp-matrix-{}", request.turn_id.as_u64())),
                finish: LlmFinish::ToolCalls,
                usage: None,
                tool_calls: vec![ObservedToolCall {
                    call_id,
                    tool_name,
                    provider_kind: Some("mcp-live-matrix".to_owned()),
                    arguments_ref,
                    native_call_ref: None,
                }],
                approval_requests: Vec::new(),
                context_token_estimate: None,
            },
        })
    }

    async fn final_result(
        &self,
        request: &LlmGenerationRequest,
        text: &str,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let content_ref = self
            .blobs
            .put_bytes(text.as_bytes().to_vec())
            .await
            .map_err(io_error)?;
        Ok(LlmGenerationResult {
            run_id: request.run_id,
            turn_id: request.turn_id,
            status: LlmGenerationStatus::Succeeded,
            failure_ref: None,
            context_entries: vec![ContextEntryInput {
                kind: ContextEntryKind::Message {
                    role: ContextMessageRole::Assistant,
                },
                content_ref,
                media_type: Some("text/plain".to_owned()),
                preview: None,
                provider_kind: Some("mcp-live-matrix".to_owned()),
                provider_item_id: None,
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!(
                    "mcp-matrix-final-{}",
                    request.turn_id.as_u64()
                )),
                finish: LlmFinish::Stop,
                usage: None,
                tool_calls: Vec::new(),
                approval_requests: Vec::new(),
                context_token_estimate: None,
            },
        })
    }
}

async fn run_matrix_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    fixture: LiveMcpFixture,
    ids: MatrixServerIds,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    put_fixture_server(
        &api,
        &ids.small,
        &fixture.small_url,
        RemoteMcpExposure::Inject,
        Some(vec!["echo".to_owned(), "rich".to_owned()]),
        RemoteMcpApprovalPolicy::Never,
    )
    .await?;
    put_fixture_server(
        &api,
        &ids.large,
        &fixture.large_url,
        RemoteMcpExposure::Search,
        None,
        RemoteMcpApprovalPolicy::Always,
    )
    .await?;
    put_fixture_server(
        &api,
        &ids.selected,
        &fixture.large_url,
        RemoteMcpExposure::Search,
        Some(vec!["catalog_tool_039".to_owned()]),
        RemoteMcpApprovalPolicy::Never,
    )
    .await?;

    assert_eq!(discover_count(&api, &ids.small).await?, 2);
    assert_eq!(discover_count(&api, &ids.large).await?, LARGE_TOOL_COUNT);
    fixture.set_large_tool_count(LARGE_TOOL_COUNT + 1).await;
    // The public discovery boundary deliberately rate-limits one server id;
    // wait out that admission cooldown before proving the second read is live.
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    assert_eq!(
        discover_count(&api, &ids.large).await?,
        LARGE_TOOL_COUNT + 1,
        "discovery must read the server again instead of a stored inventory"
    );
    fixture.set_large_tool_count(LARGE_TOOL_COUNT).await;

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            features: Some(FeaturesConfig {
                mcp: Some(api::McpFeature {
                    version: api::CURRENT_FEATURE_VERSION,
                    servers: vec![
                        McpServerLink {
                            server_id: ids.small.clone(),
                        },
                        McpServerLink {
                            server_id: ids.large.clone(),
                        },
                        McpServerLink {
                            server_id: ids.selected.clone(),
                        },
                    ],
                }),
                ..FeaturesConfig::default()
            }),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;
    let started = api
        .start_run(RunStartParams {
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "Exercise the native MCP matrix".to_owned(),
                }],
            },
            submission_id: None,
            config: Some(RunStartConfig {
                limits: Some(RunLimitsConfig {
                    max_turns: Some(12),
                    max_tool_rounds: Some(12),
                }),
                ..RunStartConfig::default()
            }),
            notify_on_terminal: None,
        })
        .await?;
    let pending = wait_for_pending_approval(&api, &session_id, &started.result.run.id).await?;
    match &pending.subject {
        api::ApprovalSubjectView::McpToolCall {
            server_id,
            tool_name,
            ..
        } => {
            assert_eq!(server_id, &ids.large);
            assert_eq!(tool_name, "catalog_tool_039");
        }
    }
    api.decide_run_approvals(RunApprovalsDecideParams {
        session_id: session_id.as_str().to_owned(),
        run_id: started.result.run.id.clone(),
        decisions: vec![ApprovalDecisionInput {
            approval_id: pending.approval_id,
            decision: ApprovalDecisionKind::Approve,
            note: Some("MCP matrix approval".to_owned()),
        }],
    })
    .await?;
    let terminal = wait_for_terminal_run(&api, &session_id, &started.result.run.id).await?;
    assert_eq!(
        final_assistant_text(&terminal),
        Some("native MCP matrix complete")
    );

    for server_id in [&ids.small, &ids.large, &ids.selected] {
        api.delete_mcp_server(McpServerDeleteParams {
            server_id: server_id.clone(),
        })
        .await?;
    }
    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("MCP live matrix cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn put_fixture_server(
    api: &GatewayAgentApi,
    server_id: &str,
    server_url: &str,
    exposure: RemoteMcpExposure,
    allowed_tools: Option<Vec<String>>,
    approval_default: RemoteMcpApprovalPolicy,
) -> anyhow::Result<()> {
    api.put_mcp_server(McpServerPutParams {
        server: McpServerInput {
            server_id: server_id.to_owned(),
            display_name: Some(format!("MCP fixture {server_id}")),
            server_url: server_url.to_owned(),
            default_server_label: server_id.to_owned(),
            description: Some("Local Streamable HTTP MCP acceptance fixture".to_owned()),
            allowed_tools,
            execution: RemoteMcpExecution::Native,
            exposure,
            approval_default,
            defer_loading_default: None,
            allow_private_network: true,
            auth_policy: api::McpServerAuthPolicy::None,
            credential: None,
            status: McpServerStatus::Active,
        },
        expected_revision: None,
    })
    .await?;
    Ok(())
}

async fn discover_count(api: &GatewayAgentApi, server_id: &str) -> anyhow::Result<usize> {
    let response = api
        .discover_mcp_server_tools(McpServerToolsDiscoverParams {
            server_id: server_id.to_owned(),
        })
        .await?;
    match response.result {
        McpServerToolsDiscoverResponse::Success { tools } => Ok(tools.len()),
        McpServerToolsDiscoverResponse::Failure { code, message, .. } => {
            anyhow::bail!("MCP discovery failed with {code:?}: {message}")
        }
    }
}

async fn wait_for_pending_approval(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    run_id: &str,
) -> anyhow::Result<api::PendingApprovalView> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let session = api
                .read_session(SessionReadParams {
                    session_id: session_id.as_str().to_owned(),
                })
                .await?
                .result
                .session;
            if let Some(run) = session.runs.iter().find(|run| run.id == run_id) {
                if let Some(approval) = run.pending_approvals.first() {
                    return Ok(approval.clone());
                }
                if matches!(
                    run.status,
                    api::RunStatus::Completed | api::RunStatus::Failed | api::RunStatus::Cancelled
                ) {
                    anyhow::bail!(
                        "MCP matrix run became terminal before approval: {:?} {:?}",
                        run.status,
                        run.tool_batches
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for MCP approval"))?
}

fn assert_find_page(
    text: &str,
    expected_len: usize,
    next_cursor: Option<u64>,
    expected_name: &str,
) -> Result<(), CoreAgentIoError> {
    let value: Value = serde_json::from_str(text).map_err(io_error)?;
    let tools = value["tools"]
        .as_array()
        .ok_or_else(|| CoreAgentIoError::Failed {
            message: format!("MCP find result has no tools array: {text}"),
        })?;
    if tools.len() != expected_len
        || value["nextCursor"].as_u64() != next_cursor
        || (!expected_name.is_empty()
            && !tools
                .iter()
                .any(|tool| tool["name"].as_str() == Some(expected_name)))
    {
        return Err(CoreAgentIoError::Failed {
            message: format!(
                "unexpected MCP find page: len={}, next={:?}, expected name={expected_name:?}, value={text}",
                tools.len(),
                value["nextCursor"].as_u64()
            ),
        });
    }
    Ok(())
}

fn require_contains(text: &str, needle: &str, step: &str) -> Result<(), CoreAgentIoError> {
    if text.contains(needle) {
        Ok(())
    } else {
        Err(CoreAgentIoError::Failed {
            message: format!("MCP matrix step {step} expected {needle:?}, got {text}"),
        })
    }
}

fn io_error(error: impl std::fmt::Display) -> CoreAgentIoError {
    CoreAgentIoError::Failed {
        message: error.to_string(),
    }
}

#[derive(Clone)]
struct LiveMcpFixture {
    state: Arc<Mutex<FixtureState>>,
    server_task: Arc<FixtureTask>,
    small_url: String,
    large_url: String,
}

struct FixtureTask(tokio::task::JoinHandle<()>);

impl Drop for FixtureTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Default)]
struct FixtureState {
    large_tool_count: usize,
    calls: Vec<String>,
}

impl LiveMcpFixture {
    async fn start() -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(FixtureState {
            large_tool_count: LARGE_TOOL_COUNT,
            calls: Vec::new(),
        }));
        let app = Router::new()
            .route("/:kind", post(fixture_handler).delete(fixture_delete))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            state,
            server_task: Arc::new(FixtureTask(task)),
            small_url: format!("http://{address}/small"),
            large_url: format!("http://{address}/large"),
        })
    }

    async fn set_large_tool_count(&self, count: usize) {
        self.state.lock().await.large_tool_count = count;
    }

    async fn assert_calls(&self) -> anyhow::Result<()> {
        let calls = self.state.lock().await.calls.clone();
        let expected = BTreeSet::from([
            "small:echo".to_owned(),
            "small:rich".to_owned(),
            "large:catalog_tool_039".to_owned(),
        ]);
        let actual = calls.into_iter().collect::<BTreeSet<_>>();
        anyhow::ensure!(
            actual == expected,
            "unexpected MCP fixture calls: {actual:?}"
        );
        let _keep_alive = &self.server_task;
        Ok(())
    }
}

async fn fixture_handler(
    Path(kind): Path<String>,
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(request): Json<Value>,
) -> Response {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            let mut response = Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": format!("lightspeed-{kind}-fixture"), "version": "1"}
                }
            }))
            .into_response();
            response.headers_mut().insert(
                "mcp-session-id",
                HeaderValue::from_static("fixture-session"),
            );
            response
        }
        Some("notifications/initialized") => StatusCode::ACCEPTED.into_response(),
        Some("tools/list") => fixture_tools_list(&kind, state, id, &request).await,
        Some("tools/call") => fixture_tool_call(&kind, state, id, &request).await,
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn fixture_tools_list(
    kind: &str,
    state: Arc<Mutex<FixtureState>>,
    id: Value,
    request: &Value,
) -> Response {
    if kind == "small" {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": [
                {
                    "name": "echo",
                    "description": "Echo a value",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"value": {"type": "string"}},
                        "required": ["value"],
                        "additionalProperties": false
                    },
                    "annotations": {"readOnlyHint": true}
                },
                {
                    "name": "rich",
                    "description": "Return text, structured data, and an image",
                    "inputSchema": {"type": "object", "additionalProperties": false}
                }
            ]}
        }))
        .into_response();
    }
    if kind != "large" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let count = state.lock().await.large_tool_count;
    let offset = request
        .pointer("/params/cursor")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let end = (offset + LARGE_PAGE_SIZE).min(count);
    let tools = (offset..end)
        .map(|index| {
            json!({
                "name": format!("catalog_tool_{index:03}"),
                "description": format!("Catalog fixture operation {index:03}"),
                "inputSchema": {
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false
                }
            })
        })
        .collect::<Vec<_>>();
    let mut result = json!({"tools": tools});
    if end < count {
        result["nextCursor"] = Value::String(end.to_string());
    }
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn fixture_tool_call(
    kind: &str,
    state: Arc<Mutex<FixtureState>>,
    id: Value,
    request: &Value,
) -> Response {
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let value = request
        .pointer("/params/arguments/value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    state.lock().await.calls.push(format!("{kind}:{name}"));
    let result = match (kind, name) {
        ("small", "echo") => json!({
            "content": [{"type": "text", "text": format!("small-echo:{value}")}]
        }),
        ("small", "rich") => json!({
            "content": [
                {"type": "text", "text": "rich-text"},
                {"type": "image", "data": "aGVsbG8=", "mimeType": "image/png"}
            ],
            "structuredContent": {"kind": "rich", "count": 1}
        }),
        ("large", name) if name.starts_with("catalog_tool_") => json!({
            "content": [{
                "type": "text",
                "text": format!("large-call:{name}:{value}")
            }]
        }),
        _ => json!({
            "content": [{"type": "text", "text": "unknown fixture tool"}],
            "isError": true
        }),
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

async fn fixture_delete() -> StatusCode {
    StatusCode::NO_CONTENT
}
