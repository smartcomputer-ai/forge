mod support;

use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use api::{
    AgentApiErrorKind, AgentApiService, AgentProfileInput, AuthProviderConfigInput,
    AuthProviderCreateParams, ContextEntryKindView, ContextMessageRoleView, EnvironmentCloseParams,
    EnvironmentCreateParams, EnvironmentCredentialBindParams, EnvironmentCredentialListParams,
    EnvironmentCredentialSourceView, EnvironmentCredentialUnbindParams, EnvironmentJobCancelParams,
    EnvironmentJobCreateParams, EnvironmentJobReadParams, EnvironmentListParams,
    EnvironmentProviderCapabilitiesView, EnvironmentProviderHeartbeatParams,
    EnvironmentProviderImplementationView, EnvironmentProviderKindView,
    EnvironmentProviderRegisterParams, EnvironmentReadParams, EnvironmentTargetDescriptorView,
    EnvironmentTargetStatusView, EnvironmentTargetSummaryView, HostCapabilitiesView,
    HostConnectionView, HostControllerConnectionView, HostScopeView, HostTargetCreateRequestView,
    HostTransportView, InputItem, ProfileCreateParams, ProfileDeleteParams, ProfileDocument,
    ProfileId, ProfileSource, RunStartParams, RunStartSource, RunStatus, SandboxTargetSpecView,
    SessionConfig, SessionConfigPutParams, SessionEnvironmentActivateParams,
    SessionEnvironmentDeactivateParams, SessionEventsReadParams, SessionJobCancelScopeView,
    SessionJobDependencyInput, SessionJobDependencyPolicyView, SessionJobHandleInput,
    SessionJobHandleView, SessionJobReadEntryView, SessionJobStartSpecInput, SessionJobStatusView,
    SessionListParams, SessionReadParams, SessionStartParams, WorkspaceLink, WorkspaceLinkAccess,
    WorkspaceLinkTarget,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use engine::{
    ContextEntryInput, ContextEntryKind, ContextEntrySource, ContextMessageRole, CoreAgentIoError,
    CoreAgentLlm, CoreAgentTools, LlmFinish, LlmGenerationFacts, LlmGenerationRequest,
    LlmGenerationResult, LlmGenerationStatus, ModelSelection, ObservedToolCall, ProviderApiKind,
    SessionId, ToolCallId, ToolName, storage::BlobStore,
};
use futures::{SinkExt, StreamExt};
use host_protocol::{
    control::{
        handshake::{ControllerCapabilities, ControllerInitializeResponse},
        methods::{
            ATTACH_TARGET_METHOD, CLOSE_TARGET_METHOD, CREATE_TARGET_METHOD,
            INITIALIZE_METHOD as CONTROL_INITIALIZE_METHOD, LIST_TARGETS_METHOD,
        },
        targets::{
            AttachTargetResponse, CloseTargetResponse, CreateTargetResponse, HostTargetStatus,
            HostTargetSummary, ListTargetsResponse,
        },
    },
    data::{
        handshake::{InitializeResponse, InitializedParams},
        methods::{
            INITIALIZE_METHOD as DATA_INITIALIZE_METHOD, INITIALIZED_METHOD, PROCESS_READ_METHOD,
            PROCESS_START_METHOD,
        },
        process::{
            ProcessOutputChunk, ProcessOutputStream, ReadProcessResponse, StartProcessParams,
            StartProcessResponse,
        },
    },
    shared::{
        ByteChunk, CURRENT_PROTOCOL_VERSION, HostCapabilities, HostConnectionId,
        HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport, ImplementationInfo,
    },
};
use serde_json::{Value, json};
use support::live::{LIVE_TEST_LOCK, final_assistant_text, require_storage_live_env};
use temporal_server::{
    gateway::{DEFAULT_MAX_REQUEST_BODY_BYTES, GatewayAgentApi, gateway_router},
    pg_store_from_env,
    worker::{ActivityState, SessionTools, WorkerActivities},
};
use temporal_workflow::AgentSessionWorkflow;
use temporalio_client::{Client, WorkflowTerminateOptions};
use tokio::{
    net::TcpListener,
    process::{Child, Command},
    task::JoinHandle,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tools::concurrency::AWAIT_TOOL_NAME;

const ATTACH_TARGET_ID: &str = "attach-target";
const CREATED_TARGET_ID: &str = "created-target";
const PROCESS_STDOUT: &str = "fake provider stdout\n";
const BRIDGE_FILE_NAME: &str = "skills/SKILL.md";
const BRIDGE_FILE_MARKER: &str = "LIGHTSPEED_BRIDGE_AGENT_MARKER";
const BRIDGE_VFS_SKILL_MARKER: &str = "LIGHTSPEED_BRIDGE_VFS_SKILL_MARKER";
const BRIDGE_JOB_FILE_NAME: &str = "job-live.txt";
const BRIDGE_JOB_MARKER: &str = "LIGHTSPEED_BRIDGE_JOB_MARKER";
const BRIDGE_JOB_SECOND_FILE_NAME: &str = "job-live-second.txt";
const BRIDGE_JOB_SECOND_MARKER: &str = "LIGHTSPEED_BRIDGE_JOB_SECOND_MARKER";
const BRIDGE_JOB_RUN_FILE_NAME: &str = "job-run-live.txt";
const BRIDGE_JOB_RUN_MARKER: &str = "LIGHTSPEED_BRIDGE_JOB_RUN_MARKER";
const BRIDGE_API_JOB_FILE_NAME: &str = "api-job-live.txt";
const BRIDGE_API_JOB_MARKER: &str = "LIGHTSPEED_BRIDGE_API_JOB_MARKER";
const BRIDGE_CREDENTIAL_ENV_NAME: &str = "P87_LIVE_TOKEN";

async fn session_events_debug(
    api: &impl AgentApiService,
    session_id: &SessionId,
) -> anyhow::Result<String> {
    let response = api
        .read_session_events(SessionEventsReadParams {
            session_id: session_id.as_str().to_owned(),
            after: None,
            limit: Some(100),
            wait_ms: Some(0),
        })
        .await?;
    Ok(format!("{:#?}", response.result.events))
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires local/up.sh or compatible Temporal + Postgres env"]
async fn temporal_live_fake_provider_create_attach_and_process_tool() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().expect("live test lock");
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let provider = FakeHostProvider::start().await?;
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(ExecCommandLlm::new(blobs.clone())) as Arc<dyn CoreAgentLlm>;
    let tools = Arc::new(SessionTools::from_pg_store(store.clone())) as Arc<dyn CoreAgentTools>;
    let activities = WorkerActivities::for_universe(
        store.config().universe_id,
        ActivityState::from_pg_store(store, llm, tools),
    );

    support::live::run_with_live_worker(activities, |client, task_queue, session_id| async move {
        run_fake_provider_client(client, task_queue, session_id, provider).await
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires local/up.sh or compatible Temporal + Postgres env"]
async fn temporal_live_profile_selects_universe_environment() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().expect("live test lock");
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let provider = FakeHostProvider::start().await?;
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(ExecCommandLlm::new(blobs.clone())) as Arc<dyn CoreAgentLlm>;
    let tools = Arc::new(SessionTools::from_pg_store(store.clone())) as Arc<dyn CoreAgentTools>;
    let activities = WorkerActivities::for_universe(
        store.config().universe_id,
        ActivityState::from_pg_store(store, llm, tools),
    );

    support::live::run_with_live_worker(activities, |client, task_queue, session_id| async move {
        run_profile_environment_client(client, task_queue, session_id, provider).await
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires local/up.sh or compatible Temporal + Postgres env and target/debug/host-bridge"]
async fn temporal_live_host_bridge_vfs_environment_isolation() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().expect("live test lock");
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let bridge_bin = host_bridge_binary_path()?;
    let bridge_root = tempfile::tempdir()?;
    let bridge_root = bridge_root.path().canonicalize()?;
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(BridgeFileLlm::new(blobs.clone())) as Arc<dyn CoreAgentLlm>;
    let tools = Arc::new(SessionTools::from_pg_store(store.clone())) as Arc<dyn CoreAgentTools>;
    let activities = WorkerActivities::for_universe(
        store.config().universe_id,
        ActivityState::from_pg_store(store, llm, tools),
    );

    support::live::run_with_live_worker(activities, |client, task_queue, session_id| async move {
        run_host_bridge_client(client, task_queue, session_id, bridge_bin, bridge_root).await
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires local/up.sh or compatible Temporal + Postgres env and target/debug/host-bridge"]
async fn temporal_live_host_bridge_environment_jobs_round_trip() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().expect("live test lock");
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let bridge_bin = host_bridge_binary_path()?;
    let bridge_root = tempfile::tempdir()?;
    let bridge_root = bridge_root.path().canonicalize()?;
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(BridgeJobsLlm::new(blobs.clone())) as Arc<dyn CoreAgentLlm>;
    let universe_id = store.config().universe_id;

    support::live::run_with_live_worker_builder(
        move |client, _task_queue| {
            let store = store.clone();
            let llm = llm.clone();
            async move {
                let tools =
                    Arc::new(SessionTools::from_pg_store(store.clone())) as Arc<dyn CoreAgentTools>;
                Ok(WorkerActivities::for_universe(
                    universe_id,
                    ActivityState::from_pg_store(store, llm, tools)
                        .with_workflow_tool_executions(client),
                ))
            }
        },
        |client, task_queue, session_id| async move {
            run_host_bridge_jobs_client(client, task_queue, session_id, bridge_bin, bridge_root)
                .await
        },
    )
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires local/up.sh or compatible Temporal + Postgres env and target/debug/host-bridge"]
async fn temporal_live_host_bridge_environment_credential_injection() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().expect("live test lock");
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let bridge_bin = host_bridge_binary_path()?;
    let bridge_root = tempfile::tempdir()?;
    let bridge_root = bridge_root.path().canonicalize()?;
    let store = pg_store_from_env().await?;
    let blobs: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(ExecCommandLlm::new(blobs.clone())) as Arc<dyn CoreAgentLlm>;
    let tools = Arc::new(SessionTools::from_pg_store(store.clone())) as Arc<dyn CoreAgentTools>;
    let activities = WorkerActivities::for_universe(
        store.config().universe_id,
        ActivityState::from_pg_store(store, llm, tools),
    );

    support::live::run_with_live_worker(activities, |client, task_queue, session_id| async move {
        run_host_bridge_credential_client(client, task_queue, session_id, bridge_bin, bridge_root)
            .await
    })
    .await
}

async fn run_host_bridge_client(
    client: Client,
    task_queue: String,
    session_id: engine::SessionId,
    bridge_bin: PathBuf,
    bridge_root: PathBuf,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let blob_store: Arc<dyn BlobStore> = store.clone();
    let model = fake_model();
    let api = Arc::new(
        GatewayAgentApi::builder(client.clone(), store)
            .with_task_queue(task_queue)
            .with_default_model(model.clone())
            .build(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let gateway_url = format!("http://{}/rpc", listener.local_addr()?);
    let gateway = tokio::spawn({
        let api = api.clone();
        async move {
            let app = gateway_router(
                std::sync::Arc::new(temporal_server::gateway::GatewayState::for_api(api)),
                DEFAULT_MAX_REQUEST_BODY_BYTES,
            );
            axum::serve(listener, app).await
        }
    });

    let provider_id = format!("host-bridge-{}", uuid::Uuid::new_v4().simple());
    let bridge = SpawnedBridge::start(&bridge_bin, &gateway_url, &provider_id, &bridge_root)?;

    let started_session = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(api_projection::model_to_api(&model)),
                features: Some(env_live_features()),
                ..SessionConfig::default()
            }),
            profile: None,
        })
        .await?;
    assert!(!started_session.result.session.managed);
    assert!(started_session.result.session.management.is_none());
    assert!(
        started_session
            .result
            .session
            .active_tools
            .tools
            .iter()
            .all(|tool| tool.tool_id != tools::environment::jobs::JOB_SUBMIT_TOOL_NAME)
    );

    let skill_snapshot = vfs::create_inline_snapshot(
        blob_store.as_ref(),
        vfs::CreateInlineSnapshotRequest::new(vec![
            vfs::InlineFile::new(
                "SKILL.md",
                format!("{BRIDGE_VFS_SKILL_MARKER}\n").into_bytes(),
            )
            .expect("inline skill"),
        ]),
    )
    .await?;
    let mut config = started_session
        .result
        .session
        .config
        .clone()
        .expect("session config");
    let features = config.features.as_mut().expect("features");
    features
        .vfs
        .as_mut()
        .expect("vfs")
        .workspace_links
        .push(WorkspaceLink {
            path: "/skills".to_owned(),
            target: WorkspaceLinkTarget::Snapshot {
                snapshot_ref: skill_snapshot.snapshot_ref.to_string(),
            },
            access: WorkspaceLinkAccess::ReadOnly,
        });
    api.put_session_config(SessionConfigPutParams {
        session_id: session_id.as_str().to_owned(),
        expected_config_revision: Some(started_session.result.session.config_revision),
        config,
    })
    .await?;

    let attached = wait_for_bridge_attach(api.as_ref(), &session_id, &provider_id).await?;
    assert!(attached.result.session.active_environment_id.is_some());
    let attached_session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    for job_tool in [
        tools::environment::jobs::JOB_SUBMIT_TOOL_NAME,
        tools::environment::jobs::JOB_READ_TOOL_NAME,
    ] {
        assert!(
            attached_session
                .result
                .session
                .active_tools
                .tools
                .iter()
                .all(|tool| tool.tool_id != job_tool),
            "environment jobs must remain default-off even for a capable provider"
        );
    }

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "read the same path from the environment and VFS domains".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = support::live::wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "host bridge filesystem isolation run did not complete: {run:#?}"
    );
    let Some(text) = final_assistant_text(&run) else {
        anyhow::bail!("host bridge isolation run missing final assistant message: {run:#?}");
    };
    assert!(
        text.contains(BRIDGE_FILE_MARKER),
        "final answer did not include marker from bridge file read: {text}"
    );
    assert!(
        text.contains(BRIDGE_VFS_SKILL_MARKER),
        "final answer did not include marker from VFS /skills read: {text}"
    );
    assert!(
        text.contains(&provider_id),
        "environment_read without an id did not return the active environment: {text}"
    );

    let local_file = bridge_root.join(BRIDGE_FILE_NAME);
    let local_contents = tokio::fs::read_to_string(&local_file).await?;
    assert!(
        local_contents.contains(BRIDGE_FILE_MARKER),
        "bridge command did not write marker to local file {}: {local_contents}",
        local_file.display()
    );

    api.deactivate_session_environment(SessionEnvironmentDeactivateParams {
        session_id: session_id.as_str().to_owned(),
    })
    .await?;
    let detached_session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(!detached_session.result.session.managed);
    assert!(detached_session.result.session.management.is_none());
    assert!(
        detached_session
            .result
            .session
            .active_tools
            .tools
            .iter()
            .all(|tool| tool.tool_id != tools::environment::jobs::JOB_SUBMIT_TOOL_NAME)
    );
    let events = api
        .read_session_events(SessionEventsReadParams {
            session_id: session_id.as_str().to_owned(),
            after: None,
            limit: Some(200),
            wait_ms: None,
        })
        .await?;
    assert!(events.result.events.iter().all(|event| !matches!(
        event.kind,
        api::SessionEventKindView::SystemWorkflowToolConfigured { .. }
    )));

    let handle = client.get_workflow_handle::<AgentSessionWorkflow>(session_id.as_str());
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("host bridge live test cleanup")
                .build(),
        )
        .await;
    drop(bridge);
    gateway.abort();
    Ok(())
}

async fn run_host_bridge_jobs_client(
    client: Client,
    task_queue: String,
    session_id: engine::SessionId,
    bridge_bin: PathBuf,
    bridge_root: PathBuf,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = fake_model();
    let api = Arc::new(
        GatewayAgentApi::builder(client.clone(), store)
            .with_task_queue(task_queue)
            .with_default_model(model.clone())
            .build(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let gateway_url = format!("http://{}/rpc", listener.local_addr()?);
    let gateway = tokio::spawn({
        let api = api.clone();
        async move {
            let app = gateway_router(
                std::sync::Arc::new(temporal_server::gateway::GatewayState::for_api(api)),
                DEFAULT_MAX_REQUEST_BODY_BYTES,
            );
            axum::serve(listener, app).await
        }
    });

    let provider_id = format!("host-bridge-jobs-{}", uuid::Uuid::new_v4().simple());
    let bridge = SpawnedBridge::start(&bridge_bin, &gateway_url, &provider_id, &bridge_root)?;

    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(api_projection::model_to_api(&model)),
                features: Some(env_live_features_with_jobs()),
                ..SessionConfig::default()
            }),
            profile: None,
        })
        .await?;
    assert!(
        started
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| tool.tool_id == tools::environment::jobs::JOB_SUBMIT_TOOL_NAME),
        "environment job tools must be installed from the feature grant before activation"
    );
    assert!(
        started
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| tool.tool_id == tools::environment::jobs::JOB_RUN_TOOL_NAME),
        "joined environment job tool must be installed from the jobs grant"
    );
    let configured = api
        .read_session_events(SessionEventsReadParams {
            session_id: session_id.as_str().to_owned(),
            after: None,
            limit: Some(200),
            wait_ms: None,
        })
        .await?;
    assert!(configured.result.events.iter().any(|event| matches!(
        event.kind,
        api::SessionEventKindView::SystemWorkflowToolConfigured { .. }
    )));

    let attached = wait_for_bridge_attach(api.as_ref(), &session_id, &provider_id).await?;
    let environment_id = attached
        .result
        .session
        .active_environment_id
        .clone()
        .expect("active bridge environment");
    let attached_session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(!attached_session.result.session.managed);
    assert!(attached_session.result.session.management.is_none());
    assert!(
        attached_session
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| tool.tool_id == tools::environment::jobs::JOB_SUBMIT_TOOL_NAME)
    );
    let listed = api.list_sessions(SessionListParams::default()).await?;
    assert!(
        !listed
            .result
            .sessions
            .iter()
            .find(|session| session.id == session_id.as_str())
            .expect("job session is listed")
            .managed
    );

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "start, wait for, and read a durable environment job".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = support::live::wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "host bridge jobs run did not complete: {run:#?}\nEvents:\n{}",
        session_events_debug(api.as_ref(), &session_id).await?
    );
    let Some(text) = final_assistant_text(&run) else {
        anyhow::bail!("host bridge jobs run missing final assistant message: {run:#?}");
    };
    assert!(
        text.contains(BRIDGE_JOB_MARKER),
        "final answer did not include marker from job output: {text}"
    );
    assert!(
        text.contains(BRIDGE_JOB_SECOND_MARKER),
        "final answer did not include marker from second job output: {text}"
    );
    assert!(
        text.contains(BRIDGE_JOB_RUN_MARKER),
        "final answer did not include marker from joined job output: {text}"
    );
    assert!(
        text.contains("\"outcome\":\"terminal\""),
        "final answer did not include a resolved await result: {text}"
    );
    let job_run_calls = run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == tools::environment::jobs::JOB_RUN_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(job_run_calls.len(), 1, "expected one joined job_run call");
    let job_run_output = job_run_calls[0]
        .output
        .as_deref()
        .expect("job_run must expose its terminal result directly");
    let job_run_value: Value = serde_json::from_str(job_run_output)?;
    assert_eq!(job_run_value["summary"]["status"], "succeeded");
    assert!(job_run_value["handle"]["environment_id"].is_string());
    assert!(job_run_value["handle"]["job_id"].is_string());
    assert!(job_run_value["output"].as_array().is_some_and(|segments| {
        segments.iter().any(|segment| {
            segment
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains(BRIDGE_JOB_RUN_MARKER))
        })
    }));
    assert!(job_run_value.get("promises").is_none());
    assert!(!job_run_output.contains("outputChunks"));
    let await_calls = run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AWAIT_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(await_calls.len(), 1, "expected one explicit await call");
    let await_call = await_calls[0];
    let await_output = await_call
        .output
        .as_deref()
        .expect("await call must expose its materialized output");
    let await_value: Value = serde_json::from_str(await_output)?;
    assert_eq!(await_value["outcome"], "terminal");
    let results = await_value["results"]
        .as_array()
        .expect("await results array");
    assert_eq!(results.len(), 2, "await must materialize both job Promises");
    assert!(results.iter().all(|result| result["status"] == "resolved"));
    assert!(results.iter().all(|result| {
        result["output"]["output"]
            .as_array()
            .is_some_and(|segments| {
                segments
                    .iter()
                    .any(|segment| segment.get("text").and_then(Value::as_str).is_some())
            })
    }));
    assert!(
        !await_output.contains("outputChunks"),
        "await must expose semantic job output rather than host transport chunks: {await_output}"
    );
    assert_eq!(
        run.entries
            .iter()
            .filter(|entry| matches!(
                &entry.kind,
                ContextEntryKindView::ToolResult { call_id, .. }
                    if call_id == &await_call.call_id
            ))
            .count(),
        1,
        "await must project one ToolResult even when it materializes two Promises"
    );
    assert_eq!(
        run.entries
            .iter()
            .filter(|entry| matches!(
                entry.kind,
                ContextEntryKindView::Message {
                    role: ContextMessageRoleView::User
                }
            ))
            .count(),
        1,
        "job Promise results must not be inserted as user messages"
    );

    let local_file = bridge_root.join(BRIDGE_JOB_FILE_NAME);
    let local_contents = tokio::fs::read_to_string(&local_file).await?;
    assert!(
        local_contents.contains(BRIDGE_JOB_MARKER),
        "bridge job did not write marker to local file {}: {local_contents}",
        local_file.display()
    );
    let second_local_file = bridge_root.join(BRIDGE_JOB_SECOND_FILE_NAME);
    let second_local_contents = tokio::fs::read_to_string(&second_local_file).await?;
    assert!(
        second_local_contents.contains(BRIDGE_JOB_SECOND_MARKER),
        "second bridge job did not write marker to local file {}: {second_local_contents}",
        second_local_file.display()
    );
    let joined_local_file = bridge_root.join(BRIDGE_JOB_RUN_FILE_NAME);
    let joined_local_contents = tokio::fs::read_to_string(&joined_local_file).await?;
    assert!(
        joined_local_contents.contains(BRIDGE_JOB_RUN_MARKER),
        "joined bridge job did not write marker to local file {}: {joined_local_contents}",
        joined_local_file.display()
    );

    let api_command = format!(
        "printf '{}\\n' > {} && printf '{}\\n'",
        BRIDGE_API_JOB_MARKER, BRIDGE_API_JOB_FILE_NAME, BRIDGE_API_JOB_MARKER
    );
    let created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.clone(),
            request_id: "api_job_round_trip".to_owned(),
            jobs: vec![SessionJobStartSpecInput {
                name: Some("api-live-job".to_owned()),
                job_id: None,
                argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), api_command],
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_ms: Some(10_000),
                depends_on: Vec::new(),
                dependency_policy: SessionJobDependencyPolicyView::AllSucceeded,
                queue_key: None,
            }],
        })
        .await?;
    assert_eq!(created.result.environment_id, environment_id);
    assert_eq!(created.result.jobs.len(), 1);
    let api_job = created.result.jobs[0].handle.clone();

    let mut api_job_output = None;
    let started = Instant::now();
    while started.elapsed() <= Duration::from_secs(10) {
        let read = api
            .read_environment_jobs(EnvironmentJobReadParams {
                jobs: vec![SessionJobHandleInput {
                    environment_id: api_job.environment_id.clone(),
                    job_id: api_job.job_id.clone(),
                }],
                output_bytes: Some(4096),
                after_seq: None,
                include_artifacts: false,
            })
            .await?;
        let entry = read.result.jobs.into_iter().next().expect("job read entry");
        if entry
            .summary
            .as_ref()
            .is_some_and(|summary| summary.status == SessionJobStatusView::Succeeded)
        {
            let output = entry
                .output_chunks
                .into_iter()
                .filter_map(|chunk| BASE64_STANDARD.decode(chunk.data_base64).ok())
                .filter_map(|bytes| String::from_utf8(bytes).ok())
                .collect::<Vec<_>>()
                .join("");
            api_job_output = Some(output);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(api_job_output) = api_job_output else {
        anyhow::bail!("environments/jobs/read did not observe API job completion");
    };
    assert!(
        api_job_output.contains(BRIDGE_API_JOB_MARKER),
        "environments/jobs/read output did not include API job marker: {api_job_output}"
    );

    let api_local_file = bridge_root.join(BRIDGE_API_JOB_FILE_NAME);
    let api_local_contents = tokio::fs::read_to_string(&api_local_file).await?;
    assert!(
        api_local_contents.contains(BRIDGE_API_JOB_MARKER),
        "API job did not write marker to local file {}: {api_local_contents}",
        api_local_file.display()
    );

    run_api_job_queue_live_check(api.as_ref(), &environment_id, &bridge_root).await?;
    run_api_job_parallel_live_check(api.as_ref(), &environment_id, &bridge_root).await?;
    run_api_job_dag_live_check(api.as_ref(), &environment_id, &bridge_root).await?;
    run_api_job_retry_live_check(api.as_ref(), &environment_id, &bridge_root).await?;

    let cancel_created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.clone(),
            request_id: "api_job_cancel".to_owned(),
            jobs: vec![SessionJobStartSpecInput {
                name: Some("api-cancel-job".to_owned()),
                job_id: None,
                argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), "sleep 30".to_owned()],
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_ms: Some(60_000),
                depends_on: Vec::new(),
                dependency_policy: SessionJobDependencyPolicyView::AllSucceeded,
                queue_key: None,
            }],
        })
        .await?;
    let cancel_job = cancel_created.result.jobs[0].handle.clone();
    let cancelled = api
        .cancel_environment_jobs(EnvironmentJobCancelParams {
            jobs: vec![SessionJobHandleInput {
                environment_id: cancel_job.environment_id.clone(),
                job_id: cancel_job.job_id.clone(),
            }],
            scope: SessionJobCancelScopeView::Job,
            force: true,
        })
        .await?;
    let cancel_status = cancelled.result.jobs[0]
        .summary
        .as_ref()
        .map(|summary| summary.status);
    assert!(
        matches!(
            cancel_status,
            Some(SessionJobStatusView::CancelRequested | SessionJobStatusView::Cancelled)
        ),
        "environments/jobs/cancel returned unexpected status: {:?}",
        cancelled.result.jobs
    );
    let cancelled_read = wait_for_environment_jobs_terminal(
        api.as_ref(),
        std::slice::from_ref(&cancel_job),
        Duration::from_secs(10),
    )
    .await?;
    assert_eq!(
        cancelled_read[0]
            .summary
            .as_ref()
            .map(|summary| summary.status),
        Some(SessionJobStatusView::Cancelled),
        "cancelled job did not reach Cancelled: {:?}",
        cancelled_read
    );

    let close_created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.clone(),
            request_id: "api_job_close_active".to_owned(),
            jobs: vec![api_shell_job("active-at-close", "sleep 30")],
        })
        .await?;
    let close_job = close_created.result.jobs[0].handle.clone();

    api.deactivate_session_environment(SessionEnvironmentDeactivateParams {
        session_id: session_id.as_str().to_owned(),
    })
    .await?;

    let close = api.close_environment(EnvironmentCloseParams {
        environment_id: environment_id.clone(),
    });
    let racing_start = api.create_environment_jobs(EnvironmentJobCreateParams {
        environment_id: environment_id.clone(),
        request_id: "api_job_close_race".to_owned(),
        jobs: vec![api_shell_job("close-race", "sleep 30")],
    });
    let (closed, racing_start) = tokio::join!(close, racing_start);
    assert_eq!(
        closed?.result.environment.status,
        EnvironmentTargetStatusView::Closed
    );

    let interrupted = wait_for_environment_jobs_terminal(
        api.as_ref(),
        std::slice::from_ref(&close_job),
        Duration::from_secs(10),
    )
    .await?;
    assert_eq!(
        interrupted[0]
            .summary
            .as_ref()
            .map(|summary| summary.status),
        Some(SessionJobStatusView::Interrupted),
        "active job was not interrupted by provider close: {interrupted:?}"
    );
    if let Ok(racing_start) = racing_start {
        let racing_job = racing_start.result.jobs[0].handle.clone();
        let racing = wait_for_environment_jobs_terminal(
            api.as_ref(),
            std::slice::from_ref(&racing_job),
            Duration::from_secs(10),
        )
        .await?;
        assert_eq!(
            racing[0].summary.as_ref().map(|summary| summary.status),
            Some(SessionJobStatusView::Interrupted),
            "job accepted during close race was not interrupted: {racing:?}"
        );
    }

    let handle = client.get_workflow_handle::<AgentSessionWorkflow>(session_id.as_str());
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("host bridge jobs live test cleanup")
                .build(),
        )
        .await;
    drop(bridge);
    gateway.abort();
    Ok(())
}

async fn run_host_bridge_credential_client(
    client: Client,
    task_queue: String,
    session_id: engine::SessionId,
    bridge_bin: PathBuf,
    bridge_root: PathBuf,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = fake_model();
    let api = Arc::new(
        GatewayAgentApi::builder(client.clone(), store)
            .with_task_queue(task_queue)
            .with_default_model(model.clone())
            .build(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let gateway_url = format!("http://{}/rpc", listener.local_addr()?);
    let gateway = tokio::spawn({
        let api = api.clone();
        async move {
            let app = gateway_router(
                std::sync::Arc::new(temporal_server::gateway::GatewayState::for_api(api)),
                DEFAULT_MAX_REQUEST_BODY_BYTES,
            );
            axum::serve(listener, app).await
        }
    });

    let provider_id = format!("host-bridge-credential-{}", uuid::Uuid::new_v4().simple());
    let credential_provider_id = format!("p87-credential-{}", uuid::Uuid::new_v4().simple());
    let secret_value = format!("p87-live-secret-{}", uuid::Uuid::new_v4().simple());
    let bridge = SpawnedBridge::start(&bridge_bin, &gateway_url, &provider_id, &bridge_root)?;

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(api_projection::model_to_api(&model)),
            features: Some(env_live_features()),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    let attached = wait_for_bridge_attach(api.as_ref(), &session_id, &provider_id).await?;
    let environment_id = attached
        .result
        .session
        .active_environment_id
        .clone()
        .expect("active bridge environment");

    let provider = api
        .create_auth_provider(AuthProviderCreateParams {
            provider_id: Some(credential_provider_id.clone()),
            display_name: Some("P87 live credential".to_owned()),
            config: AuthProviderConfigInput::ModelApiKey {},
            credential: Some(secret_value.clone()),
        })
        .await?;
    assert_eq!(provider.result.provider.provider_id, credential_provider_id);
    assert!(provider.result.provider.has_credential);

    let bound = api
        .bind_environment_credential(EnvironmentCredentialBindParams {
            environment_id: environment_id.clone(),
            env_name: BRIDGE_CREDENTIAL_ENV_NAME.to_owned(),
            source: EnvironmentCredentialSourceView::AuthProviderCredential {
                provider_id: credential_provider_id.clone(),
            },
        })
        .await?;
    assert_eq!(bound.result.credential.env_name, BRIDGE_CREDENTIAL_ENV_NAME);

    let listed = api
        .list_environment_credentials(EnvironmentCredentialListParams {
            environment_id: environment_id.clone(),
        })
        .await?;
    assert!(
        listed
            .result
            .credentials
            .iter()
            .any(|credential| credential.env_name == BRIDGE_CREDENTIAL_ENV_NAME),
        "credential binding was not listed after bind: {:?}",
        listed.result.credentials
    );

    let created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.clone(),
            request_id: "api_environment_credential_injection".to_owned(),
            jobs: vec![api_shell_job(
                "credential-injection",
                format!(
                    "test -n \"${{{BRIDGE_CREDENTIAL_ENV_NAME}:-}}\" && printf '%s' \"${BRIDGE_CREDENTIAL_ENV_NAME}\""
                ),
            )],
        })
        .await?;
    let entries = wait_for_environment_jobs_terminal(
        api.as_ref(),
        &[created.result.jobs[0].handle.clone()],
        Duration::from_secs(10),
    )
    .await?;
    ensure_job_statuses(
        &entries,
        SessionJobStatusView::Succeeded,
        "credential-injected raw environment job",
    )?;
    let output = entries[0]
        .output_chunks
        .iter()
        .map(|chunk| BASE64_STANDARD.decode(&chunk.data_base64))
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    assert_eq!(output, b"<redacted>");
    assert!(!String::from_utf8_lossy(&output).contains(&secret_value));

    let unbound = api
        .unbind_environment_credential(EnvironmentCredentialUnbindParams {
            environment_id,
            env_name: BRIDGE_CREDENTIAL_ENV_NAME.to_owned(),
        })
        .await?;
    assert_eq!(
        unbound.result.credential.env_name,
        BRIDGE_CREDENTIAL_ENV_NAME
    );

    api.deactivate_session_environment(SessionEnvironmentDeactivateParams {
        session_id: session_id.as_str().to_owned(),
    })
    .await?;

    let handle = client.get_workflow_handle::<AgentSessionWorkflow>(session_id.as_str());
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("host bridge credential live test cleanup")
                .build(),
        )
        .await;
    drop(bridge);
    gateway.abort();
    Ok(())
}

async fn run_api_job_queue_live_check(
    api: &GatewayAgentApi,
    environment_id: &str,
    bridge_root: &std::path::Path,
) -> anyhow::Result<()> {
    let queue_file_name = "api-queue-order.txt";
    let queue_file = bridge_root.join(queue_file_name);
    let mut first = api_shell_job("queue-1", format!("printf 1 >> {queue_file_name}"));
    let mut second = api_shell_job("queue-2", format!("printf 2 >> {queue_file_name}"));
    let mut third = api_shell_job("queue-3", format!("printf 3 >> {queue_file_name}"));
    first.queue_key = Some("api_live_queue".to_owned());
    second.queue_key = Some("api_live_queue".to_owned());
    third.queue_key = Some("api_live_queue".to_owned());

    let created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.to_owned(),
            request_id: "api_live_queue".to_owned(),
            jobs: vec![first, second, third],
        })
        .await?;
    let handles = created
        .result
        .jobs
        .iter()
        .map(|job| job.handle.clone())
        .collect::<Vec<_>>();
    let entries =
        wait_for_environment_jobs_terminal(api, &handles, Duration::from_secs(15)).await?;
    ensure_job_statuses(
        &entries,
        SessionJobStatusView::Succeeded,
        "queue-keyed jobs",
    )?;
    let contents = tokio::fs::read_to_string(&queue_file).await?;
    assert_eq!(
        contents, "123",
        "queue-keyed jobs did not execute serially in accepted order"
    );
    Ok(())
}

async fn run_api_job_parallel_live_check(
    api: &GatewayAgentApi,
    environment_id: &str,
    bridge_root: &std::path::Path,
) -> anyhow::Result<()> {
    let order_file_name = "api-parallel-order.txt";
    let order_file = bridge_root.join(order_file_name);
    let created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.to_owned(),
            request_id: "api_live_parallel".to_owned(),
            jobs: vec![
                api_shell_job(
                    "parallel-a",
                    format!(
                        "printf 'a-start\\n' >> {order_file_name}; sleep 1; printf 'a-end\\n' >> {order_file_name}"
                    ),
                ),
                api_shell_job(
                    "parallel-b",
                    format!(
                        "printf 'b-start\\n' >> {order_file_name}; sleep 1; printf 'b-end\\n' >> {order_file_name}"
                    ),
                ),
            ],
        })
        .await?;
    let handles = created
        .result
        .jobs
        .iter()
        .map(|job| job.handle.clone())
        .collect::<Vec<_>>();
    let entries =
        wait_for_environment_jobs_terminal(api, &handles, Duration::from_secs(15)).await?;
    ensure_job_statuses(&entries, SessionJobStatusView::Succeeded, "parallel jobs")?;

    let contents = tokio::fs::read_to_string(&order_file).await?;
    let lines = contents.lines().collect::<Vec<_>>();
    let a_start = line_index(&lines, "a-start")?;
    let b_start = line_index(&lines, "b-start")?;
    let a_end = line_index(&lines, "a-end")?;
    let b_end = line_index(&lines, "b-end")?;
    let latest_start = a_start.max(b_start);
    let earliest_end = a_end.min(b_end);
    assert!(
        latest_start < earliest_end,
        "parallel jobs did not overlap; order file was: {contents:?}"
    );
    Ok(())
}

async fn run_api_job_dag_live_check(
    api: &GatewayAgentApi,
    environment_id: &str,
    bridge_root: &std::path::Path,
) -> anyhow::Result<()> {
    let dag_file_name = "api-dag-order.txt";
    let dag_file = bridge_root.join(dag_file_name);
    let checkout = api_shell_job("checkout", format!("printf A >> {dag_file_name}"));
    let mut build = api_shell_job("build", format!("printf B >> {dag_file_name}"));
    build.depends_on = vec![SessionJobDependencyInput {
        job_id: None,
        name: Some("checkout".to_owned()),
    }];
    let mut tests = api_shell_job("tests", format!("printf C >> {dag_file_name}"));
    tests.depends_on = vec![SessionJobDependencyInput {
        job_id: None,
        name: Some("build".to_owned()),
    }];

    let created = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.to_owned(),
            request_id: "api_live_dag".to_owned(),
            jobs: vec![checkout, build, tests],
        })
        .await?;
    let final_handle = created
        .result
        .jobs
        .last()
        .expect("created DAG final job")
        .handle
        .clone();
    let entries = wait_for_environment_jobs_terminal(
        api,
        std::slice::from_ref(&final_handle),
        Duration::from_secs(15),
    )
    .await?;
    ensure_job_statuses(
        &entries,
        SessionJobStatusView::Succeeded,
        "dependency DAG final job",
    )?;
    let contents = tokio::fs::read_to_string(&dag_file).await?;
    assert_eq!(
        contents, "ABC",
        "dependency DAG did not execute in dependency order"
    );
    Ok(())
}

async fn run_api_job_retry_live_check(
    api: &GatewayAgentApi,
    environment_id: &str,
    bridge_root: &std::path::Path,
) -> anyhow::Result<()> {
    let retry_file_name = "api-retry-count.txt";
    let retry_file = bridge_root.join(retry_file_name);
    let params = EnvironmentJobCreateParams {
        environment_id: environment_id.to_owned(),
        request_id: "api_live_retry".to_owned(),
        jobs: vec![api_shell_job(
            "retry",
            format!("printf R >> {retry_file_name}"),
        )],
    };

    let first = api.create_environment_jobs(params.clone()).await?;
    let second = api.create_environment_jobs(params).await?;
    assert_eq!(
        first.result.jobs[0].handle.job_id, second.result.jobs[0].handle.job_id,
        "retry-stable API start did not return the same job id"
    );
    let handle = first.result.jobs[0].handle.clone();
    let entries = wait_for_environment_jobs_terminal(
        api,
        std::slice::from_ref(&handle),
        Duration::from_secs(10),
    )
    .await?;
    ensure_job_statuses(&entries, SessionJobStatusView::Succeeded, "retry job")?;

    let contents = tokio::fs::read_to_string(&retry_file).await?;
    assert_eq!(
        contents, "R",
        "retry-stable API start executed the job more than once"
    );

    let conflict = api
        .create_environment_jobs(EnvironmentJobCreateParams {
            environment_id: environment_id.to_owned(),
            request_id: "api_live_retry".to_owned(),
            jobs: vec![api_shell_job(
                "retry",
                format!("printf X >> {retry_file_name}"),
            )],
        })
        .await;
    assert!(
        conflict.is_err(),
        "same request/job identity with different input must be rejected"
    );
    assert_eq!(
        tokio::fs::read_to_string(&retry_file).await?,
        "R",
        "conflicting retry must not execute"
    );
    Ok(())
}

fn api_shell_job(name: &str, shell: impl Into<String>) -> SessionJobStartSpecInput {
    SessionJobStartSpecInput {
        name: Some(name.to_owned()),
        job_id: None,
        argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), shell.into()],
        cwd: None,
        env: BTreeMap::new(),
        stdin: None,
        timeout_ms: Some(10_000),
        depends_on: Vec::new(),
        dependency_policy: SessionJobDependencyPolicyView::AllSucceeded,
        queue_key: None,
    }
}

async fn wait_for_environment_jobs_terminal(
    api: &GatewayAgentApi,
    handles: &[SessionJobHandleView],
    timeout: Duration,
) -> anyhow::Result<Vec<SessionJobReadEntryView>> {
    let started = Instant::now();
    loop {
        let read = api
            .read_environment_jobs(EnvironmentJobReadParams {
                jobs: handles.iter().map(session_job_handle_input).collect(),
                output_bytes: Some(4096),
                after_seq: None,
                include_artifacts: false,
            })
            .await?;
        if read.result.jobs.len() != handles.len() {
            anyhow::bail!(
                "environments/jobs/read returned {} entries for {} handles",
                read.result.jobs.len(),
                handles.len()
            );
        }
        for entry in &read.result.jobs {
            if let Some(error) = entry.error.as_deref() {
                anyhow::bail!("environments/jobs/read returned entry error: {error}");
            }
        }
        if read.result.jobs.iter().all(|entry| {
            entry
                .summary
                .as_ref()
                .is_some_and(|summary| is_terminal_job_status(summary.status))
        }) {
            return Ok(read.result.jobs);
        }
        if started.elapsed() > timeout {
            anyhow::bail!(
                "environment jobs did not reach terminal status within {:?}: {:?}",
                timeout,
                job_status_debug(&read.result.jobs)
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn session_job_handle_input(handle: &SessionJobHandleView) -> SessionJobHandleInput {
    SessionJobHandleInput {
        environment_id: handle.environment_id.clone(),
        job_id: handle.job_id.clone(),
    }
}

fn is_terminal_job_status(status: SessionJobStatusView) -> bool {
    matches!(
        status,
        SessionJobStatusView::Succeeded
            | SessionJobStatusView::Failed
            | SessionJobStatusView::Cancelled
            | SessionJobStatusView::TimedOut
            | SessionJobStatusView::DependencyFailed
            | SessionJobStatusView::Interrupted
            | SessionJobStatusView::Lost
    )
}

fn ensure_job_statuses(
    entries: &[SessionJobReadEntryView],
    expected: SessionJobStatusView,
    label: &str,
) -> anyhow::Result<()> {
    let statuses = job_status_debug(entries);
    if entries.iter().all(|entry| {
        entry
            .summary
            .as_ref()
            .is_some_and(|summary| summary.status == expected)
    }) {
        return Ok(());
    }
    anyhow::bail!("{label} did not all finish as {expected:?}: {statuses:?}")
}

fn job_status_debug(entries: &[SessionJobReadEntryView]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry.summary.as_ref() {
            Some(summary) => format!("{}:{:?}", summary.job_id, summary.status),
            None => format!("missing-summary:{:?}", entry.error),
        })
        .collect()
}

fn line_index(lines: &[&str], expected: &str) -> anyhow::Result<usize> {
    lines
        .iter()
        .position(|line| *line == expected)
        .ok_or_else(|| anyhow::anyhow!("missing {expected:?} in {lines:?}"))
}

async fn run_fake_provider_client(
    client: Client,
    task_queue: String,
    session_id: engine::SessionId,
    provider: FakeHostProvider,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = fake_model();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let provider_id = format!("fake-provider-{}", uuid::Uuid::new_v4().simple());

    let registered = api
        .register_environment_provider(EnvironmentProviderRegisterParams {
            provider_id: provider_id.clone(),
            provider_kind: EnvironmentProviderKindView::Bridge,
            controller_connection: HostControllerConnectionView {
                endpoint: provider.endpoint().to_owned(),
                transport: HostTransportView::WebSocket,
            },
            capabilities: EnvironmentProviderCapabilitiesView::default(),
            implementation: EnvironmentProviderImplementationView {
                name: "client-supplied-placeholder".to_owned(),
                version: None,
            },
            lease_ttl_ms: 60_000,
            display_name: Some("fake host provider".to_owned()),
            metadata: BTreeMap::new(),
        })
        .await?;
    assert!(registered.result.provider.capabilities.create_target);
    assert_eq!(
        registered.result.provider.implementation.name,
        "fake-host-provider"
    );
    assert_eq!(provider.controller_initialize_count(), 1);

    let heartbeat = api
        .heartbeat_environment_provider(EnvironmentProviderHeartbeatParams {
            provider_id: provider_id.clone(),
            lease_ttl_ms: None,
            observed_targets: vec![target_descriptor(provider.endpoint(), ATTACH_TARGET_ID)],
        })
        .await?;
    assert_eq!(heartbeat.result.environments.len(), 1);
    assert_eq!(
        heartbeat.result.environments[0].provider_target_id,
        ATTACH_TARGET_ID
    );
    let attach_environment_id = heartbeat.result.environments[0].environment_id.clone();

    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(api_projection::model_to_api(&model)),
                features: Some(env_live_features()),
                ..SessionConfig::default()
            }),
            profile: None,
        })
        .await?;
    assert!(
        started
            .result
            .session
            .active_context
            .entries
            .iter()
            .any(|entry| entry.kind == ContextEntryKindView::VfsCatalog)
    );

    let attached = api
        .activate_session_environment(SessionEnvironmentActivateParams {
            session_id: session_id.as_str().to_owned(),
            environment_id: attach_environment_id,
        })
        .await?;
    assert!(attached.result.session.active_environment_id.is_some());
    assert_eq!(provider.attach_count(), 0);
    let session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(session.result.session.active_environment_id.is_some());

    let first = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run a command in the attached provider target".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let first_run =
        support::live::wait_for_terminal_run(&api, &session_id, &first.result.run.id).await?;
    assert_eq!(
        first_run.status,
        RunStatus::Completed,
        "first run did not complete: {first_run:#?}"
    );
    let Some(first_text) = final_assistant_text(&first_run) else {
        anyhow::bail!("first run missing final assistant message: {first_run:#?}");
    };
    assert!(first_text.contains(PROCESS_STDOUT));

    api.deactivate_session_environment(SessionEnvironmentDeactivateParams {
        session_id: session_id.as_str().to_owned(),
    })
    .await?;
    assert_eq!(
        provider.close_count(),
        0,
        "bridge detach should not close target when close_target=false"
    );

    let created = api
        .create_environment(EnvironmentCreateParams {
            provider_id: provider_id.clone(),
            request: HostTargetCreateRequestView::Sandbox {
                spec: SandboxTargetSpecView {
                    image: Some("fake-image".to_owned()),
                    cwd: Some("/workspace".to_owned()),
                    ..SandboxTargetSpecView::default()
                },
            },
        })
        .await?;
    let created_environment_id = created.result.environment.environment_id.clone();
    let created = api
        .activate_session_environment(SessionEnvironmentActivateParams {
            session_id: session_id.as_str().to_owned(),
            environment_id: created_environment_id.clone(),
        })
        .await?;
    assert_eq!(
        created.result.session.active_environment_id.as_ref(),
        Some(&created_environment_id)
    );
    assert_eq!(provider.create_count(), 1);

    let second = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run a command in the created provider target".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let second_run =
        support::live::wait_for_terminal_run(&api, &session_id, &second.result.run.id).await?;
    assert_eq!(
        second_run.status,
        RunStatus::Completed,
        "second run did not complete: {second_run:#?}"
    );
    let Some(second_text) = final_assistant_text(&second_run) else {
        anyhow::bail!("second run missing final assistant message: {second_run:#?}");
    };
    assert!(second_text.contains(PROCESS_STDOUT));

    provider.reject_next_close();
    let rejected = api
        .close_environment(EnvironmentCloseParams {
            environment_id: created_environment_id.clone(),
        })
        .await
        .expect_err("provider should reject the first close");
    assert_eq!(rejected.kind, AgentApiErrorKind::Rejected);
    let restored = api
        .read_environment(EnvironmentReadParams {
            environment_id: created_environment_id.clone(),
        })
        .await?;
    assert_eq!(
        restored.result.environment.status,
        EnvironmentTargetStatusView::Ready
    );
    api.close_environment(EnvironmentCloseParams {
        environment_id: created_environment_id,
    })
    .await?;
    assert_eq!(provider.close_count(), 2);
    assert_eq!(provider.process_start_count(), 2);
    assert_eq!(
        provider.process_cwds(),
        vec![Some("/workspace".to_owned()), Some("/workspace".to_owned())]
    );

    let handle = client.get_workflow_handle::<AgentSessionWorkflow>(session_id.as_str());
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("fake provider live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_profile_environment_client(
    client: Client,
    task_queue: String,
    session_id: engine::SessionId,
    provider: FakeHostProvider,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = fake_model();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let provider_id = format!("profile-provider-{}", uuid::Uuid::new_v4().simple());
    let profile_id = ProfileId::new(format!("profile_env_{}", uuid::Uuid::new_v4().simple()));

    api.register_environment_provider(EnvironmentProviderRegisterParams {
        provider_id: provider_id.clone(),
        provider_kind: EnvironmentProviderKindView::Bridge,
        controller_connection: HostControllerConnectionView {
            endpoint: provider.endpoint().to_owned(),
            transport: HostTransportView::WebSocket,
        },
        capabilities: EnvironmentProviderCapabilitiesView::default(),
        implementation: EnvironmentProviderImplementationView {
            name: "client-supplied-placeholder".to_owned(),
            version: None,
        },
        lease_ttl_ms: 60_000,
        display_name: Some("profile fake host provider".to_owned()),
        metadata: BTreeMap::new(),
    })
    .await?;

    let heartbeat = api
        .heartbeat_environment_provider(EnvironmentProviderHeartbeatParams {
            provider_id: provider_id.clone(),
            lease_ttl_ms: None,
            observed_targets: vec![target_descriptor(provider.endpoint(), ATTACH_TARGET_ID)],
        })
        .await?;
    let environment_id = heartbeat.result.environments[0].environment_id.clone();

    api.create_profile(ProfileCreateParams {
        profile: AgentProfileInput {
            profile_id: profile_id.clone(),
            display_name: Some("Profile environment".to_owned()),
            description: Some("Select fake host provider environment".to_owned()),
            document: ProfileDocument {
                config: Some(SessionConfig {
                    model: Some(api_projection::model_to_api(&model)),
                    features: Some(env_live_features()),
                    ..SessionConfig::default()
                }),
                instructions: None,
                active_environment_id: Some(environment_id.clone()),
            },
        },
    })
    .await?;

    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: None,
            profile: Some(ProfileSource::Named {
                profile_id: profile_id.clone(),
            }),
        })
        .await?;
    assert_eq!(started.result.session.id, session_id.as_str());
    assert_eq!(provider.attach_count(), 0);

    assert_eq!(
        started.result.session.active_environment_id.as_ref(),
        Some(&environment_id)
    );

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run a command in the profile attached provider target".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = support::live::wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    assert_eq!(
        run.status,
        RunStatus::Completed,
        "profile environment run did not complete: {run:#?}"
    );
    let Some(text) = final_assistant_text(&run) else {
        anyhow::bail!("profile environment run missing final assistant message: {run:#?}");
    };
    assert!(text.contains(PROCESS_STDOUT));

    api.deactivate_session_environment(SessionEnvironmentDeactivateParams {
        session_id: session_id.as_str().to_owned(),
    })
    .await?;
    api.delete_profile(ProfileDeleteParams { profile_id })
        .await?;

    let handle = client.get_workflow_handle::<AgentSessionWorkflow>(session_id.as_str());
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("profile environment live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

fn fake_model() -> ModelSelection {
    ModelSelection {
        api_kind: ProviderApiKind::OpenAiResponses,
        provider_id: "fake".to_owned(),
        model: "fake-env-tool-model".to_owned(),
    }
}

struct ExecCommandLlm {
    blobs: Arc<dyn BlobStore>,
}

impl ExecCommandLlm {
    fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self { blobs }
    }

    async fn tool_call_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if !request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "exec_command")
        {
            return Err(io_error("planned request did not expose exec_command"));
        }
        let arguments = json!({
            "argv": ["fake-provider-command"],
            "yield_time_ms": 1,
            "max_output_bytes": 4096
        });
        let arguments_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(&arguments).map_err(io_error)?)
            .await
            .map_err(io_error)?;
        let call_id = ToolCallId::new(format!("env_call_{}_{}", request.run_id, request.turn_id));
        let tool_name = ToolName::new("exec_command");
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
                preview: Some(format!("exec_command({arguments})")),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: Some(call_id.as_str().to_owned()),
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("fake-tool-{}", request.turn_id)),
                finish: LlmFinish::ToolCalls,
                usage: None,
                tool_calls: vec![ObservedToolCall {
                    call_id,
                    tool_name,
                    provider_kind: Some("fake".to_owned()),
                    arguments_ref,
                    native_call_ref: None,
                }],
                context_token_estimate: None,
            },
        })
    }

    async fn final_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let tool_output = if let Some(entry) = current_run_tool_result(request) {
            self.blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?
        } else {
            "no tool result".to_owned()
        };
        let text = format!("Fake provider run completed with output:\n{tool_output}");
        let output_ref = self
            .blobs
            .put_bytes(text.into_bytes())
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
                content_ref: output_ref,
                media_type: Some("text/plain".to_owned()),
                preview: Some("fake provider final answer".to_owned()),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: None,
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("fake-final-{}", request.turn_id)),
                finish: LlmFinish::Stop,
                usage: None,
                tool_calls: Vec::new(),
                context_token_estimate: None,
            },
        })
    }
}

#[async_trait]
impl CoreAgentLlm for ExecCommandLlm {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if current_run_tool_result(&request).is_some() {
            self.final_result(&request).await
        } else {
            self.tool_call_result(&request).await
        }
    }
}

struct BridgeFileLlm {
    blobs: Arc<dyn BlobStore>,
}

impl BridgeFileLlm {
    fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self { blobs }
    }

    async fn read_environment_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if !request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "environment_read")
        {
            return Err(io_error("planned request did not expose environment_read"));
        }
        self.tool_call_result(
            request,
            "environment_read",
            json!({}),
            "bridge_read_environment",
        )
        .await
    }

    async fn exec_write_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if !request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "exec_command")
        {
            return Err(io_error("planned request did not expose exec_command"));
        }
        let command = format!(
            "mkdir -p skills && printf '{} from exec_command\\n' > {} && printf 'wrote {}\\n'",
            BRIDGE_FILE_MARKER, BRIDGE_FILE_NAME, BRIDGE_FILE_NAME
        );
        self.tool_call_result(
            request,
            "exec_command",
            json!({
                "argv": ["/bin/sh", "-c", command],
                "timeout_ms": 5000,
                "max_output_bytes": 4096
            }),
            "bridge_exec_write",
        )
        .await
    }

    async fn read_file_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if !request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "read_file")
        {
            return Err(io_error("planned request did not expose read_file"));
        }
        self.tool_call_result(
            request,
            "read_file",
            json!({
                "path": BRIDGE_FILE_NAME,
                "offset": 1,
                "limit": 20
            }),
            "bridge_read_file",
        )
        .await
    }

    async fn read_vfs_skill_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        if !request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == "vfs_read_file")
        {
            return Err(io_error("planned request did not expose vfs_read_file"));
        }
        self.tool_call_result(
            request,
            "vfs_read_file",
            json!({
                "path": BRIDGE_FILE_NAME,
                "offset": 1,
                "limit": 20
            }),
            "bridge_read_vfs_skill",
        )
        .await
    }

    async fn tool_call_result(
        &self,
        request: &LlmGenerationRequest,
        tool_name: &str,
        arguments: Value,
        label: &str,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let arguments_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(&arguments).map_err(io_error)?)
            .await
            .map_err(io_error)?;
        let call_id = ToolCallId::new(format!("{label}_{}_{}", request.run_id, request.turn_id));
        let tool_name = ToolName::new(tool_name);
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
                preview: Some(format!("{}({arguments})", tool_name.as_str())),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: Some(call_id.as_str().to_owned()),
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("fake-{label}-{}", request.turn_id)),
                finish: LlmFinish::ToolCalls,
                usage: None,
                tool_calls: vec![ObservedToolCall {
                    call_id,
                    tool_name,
                    provider_kind: Some("fake".to_owned()),
                    arguments_ref,
                    native_call_ref: None,
                }],
                context_token_estimate: None,
            },
        })
    }

    async fn final_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let mut text = String::from("Host bridge filesystem isolation test completed.\n");
        for entry in current_run_tool_results(request) {
            let output = self
                .blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?;
            text.push_str("\n--- tool result ---\n");
            text.push_str(&output);
        }
        let output_ref = self
            .blobs
            .put_bytes(text.into_bytes())
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
                content_ref: output_ref,
                media_type: Some("text/plain".to_owned()),
                preview: Some("host bridge final answer".to_owned()),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: None,
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("fake-host-bridge-final-{}", request.turn_id)),
                finish: LlmFinish::Stop,
                usage: None,
                tool_calls: Vec::new(),
                context_token_estimate: None,
            },
        })
    }
}

#[async_trait]
impl CoreAgentLlm for BridgeFileLlm {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        match current_run_tool_results(&request).len() {
            0 => self.read_environment_result(&request).await,
            1 => self.exec_write_result(&request).await,
            2 => self.read_file_result(&request).await,
            3 => self.read_vfs_skill_result(&request).await,
            _ => self.final_result(&request).await,
        }
    }
}

struct BridgeJobsLlm {
    blobs: Arc<dyn BlobStore>,
}

impl BridgeJobsLlm {
    fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self { blobs }
    }

    async fn run_joined_job_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        self.require_tool(request, tools::environment::jobs::JOB_RUN_TOOL_NAME)?;
        let command = format!(
            "printf '{}\\n' > {} && printf '{}\\n'",
            BRIDGE_JOB_RUN_MARKER, BRIDGE_JOB_RUN_FILE_NAME, BRIDGE_JOB_RUN_MARKER
        );
        self.tool_call_result(
            request,
            tools::environment::jobs::JOB_RUN_TOOL_NAME,
            json!({
                "name": "live-joined-job",
                "argv": ["/bin/sh", "-c", command],
                "timeout_ms": 10000
            }),
            "bridge_job_run",
        )
        .await
    }

    async fn submit_job_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        self.require_tool(request, "job_submit")?;
        let command = format!(
            "printf '{}\\n' > {} && printf '{}\\n'",
            BRIDGE_JOB_MARKER, BRIDGE_JOB_FILE_NAME, BRIDGE_JOB_MARKER
        );
        let second_command = format!(
            "printf '{}\\n' > {} && printf '{}\\n'",
            BRIDGE_JOB_SECOND_MARKER, BRIDGE_JOB_SECOND_FILE_NAME, BRIDGE_JOB_SECOND_MARKER
        );
        self.tool_call_result(
            request,
            "job_submit",
            json!({
                "jobs": [{
                    "name": "live-job",
                    "job_id": "live-job",
                    "argv": ["/bin/sh", "-c", command],
                    "timeout_ms": 10000
                }, {
                    "name": "live-job-second",
                    "job_id": "live-job-second",
                    "argv": ["/bin/sh", "-c", second_command],
                    "timeout_ms": 10000
                }]
            }),
            "bridge_job_submit",
        )
        .await
    }

    async fn await_job_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        self.require_tool(request, AWAIT_TOOL_NAME)?;
        let promise_ids = self.job_promises_from_results(request).await?;
        self.tool_call_result(
            request,
            AWAIT_TOOL_NAME,
            json!({
                "promises": promise_ids,
                "mode": "all",
                "timeout_ms": 15000
            }),
            "bridge_job_await",
        )
        .await
    }

    async fn read_job_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        self.require_tool(request, "job_read")?;
        let handles = self.job_handles_from_results(request).await?;
        self.tool_call_result(
            request,
            "job_read",
            json!({
                "jobs": handles.into_iter().map(|handle| handle.json_arg()).collect::<Vec<_>>(),
                "output_bytes": 4096
            }),
            "bridge_job_read",
        )
        .await
    }

    fn require_tool(
        &self,
        request: &LlmGenerationRequest,
        name: &str,
    ) -> Result<(), CoreAgentIoError> {
        if request
            .request
            .tools
            .iter()
            .any(|tool| tool.name.as_str() == name)
        {
            return Ok(());
        }
        Err(io_error(format!("planned request did not expose {name}")))
    }

    async fn tool_call_result(
        &self,
        request: &LlmGenerationRequest,
        tool_name: &str,
        arguments: Value,
        label: &str,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let arguments_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(&arguments).map_err(io_error)?)
            .await
            .map_err(io_error)?;
        let call_id = ToolCallId::new(format!("{label}_{}_{}", request.run_id, request.turn_id));
        let tool_name = ToolName::new(tool_name);
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
                preview: Some(format!("{}({arguments})", tool_name.as_str())),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: Some(call_id.as_str().to_owned()),
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("fake-{label}-{}", request.turn_id)),
                finish: LlmFinish::ToolCalls,
                usage: None,
                tool_calls: vec![ObservedToolCall {
                    call_id,
                    tool_name,
                    provider_kind: Some("fake".to_owned()),
                    arguments_ref,
                    native_call_ref: None,
                }],
                context_token_estimate: None,
            },
        })
    }

    async fn job_handles_from_results(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<Vec<BridgeJobHandle>, CoreAgentIoError> {
        let mut handles = Vec::new();
        for entry in current_run_tool_results(request).into_iter().rev() {
            let output = self
                .blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?;
            for line in output.lines() {
                if let Some(handle) = BridgeJobHandle::parse(line) {
                    push_unique_job_handle(&mut handles, handle);
                }
            }
        }
        // Once `await` resolves, its aggregate tool result carries each
        // Promise's semantic job result and stable provider handle.
        for entry in current_run_tool_results(request).into_iter().rev() {
            let output = self
                .blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?;
            for handle in BridgeJobHandle::parse_await_payload(&output) {
                push_unique_job_handle(&mut handles, handle);
            }
        }
        if handles.len() == 2 {
            Ok(handles)
        } else {
            Err(io_error(format!(
                "job_submit result included {} job handles, expected 2",
                handles.len()
            )))
        }
    }

    async fn job_promises_from_results(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<Vec<String>, CoreAgentIoError> {
        for entry in current_run_tool_results(request).into_iter().rev() {
            let output = self
                .blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?;
            let promise_ids = parse_job_promises(&output);
            if promise_ids.len() == 2 {
                return Ok(promise_ids);
            }
        }
        Err(io_error(
            "job_submit result did not include two keyed promises",
        ))
    }

    async fn final_result(
        &self,
        request: &LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let mut text = String::from("Host bridge durable job test completed.\n");
        for entry in current_run_tool_results(request) {
            let output = self
                .blobs
                .read_text(&entry.content_ref)
                .await
                .map_err(io_error)?;
            text.push_str("\n--- tool result ---\n");
            text.push_str(&output);
        }
        let output_ref = self
            .blobs
            .put_bytes(text.into_bytes())
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
                content_ref: output_ref,
                media_type: Some("text/plain".to_owned()),
                preview: Some("host bridge jobs final answer".to_owned()),
                provider_kind: Some("fake".to_owned()),
                provider_item_id: None,
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!(
                    "fake-host-bridge-jobs-final-{}",
                    request.turn_id
                )),
                finish: LlmFinish::Stop,
                usage: None,
                tool_calls: Vec::new(),
                context_token_estimate: None,
            },
        })
    }
}

#[async_trait]
impl CoreAgentLlm for BridgeJobsLlm {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        match current_run_tool_results(&request).len() {
            0 => self.run_joined_job_result(&request).await,
            1 => self.submit_job_result(&request).await,
            2 => self.await_job_result(&request).await,
            3 => self.read_job_result(&request).await,
            _ => self.final_result(&request).await,
        }
    }
}

struct BridgeJobHandle {
    environment_id: String,
    job_id: String,
}

fn push_unique_job_handle(handles: &mut Vec<BridgeJobHandle>, candidate: BridgeJobHandle) {
    if !handles.iter().any(|handle| {
        handle.environment_id == candidate.environment_id && handle.job_id == candidate.job_id
    }) {
        handles.push(candidate);
    }
}

impl BridgeJobHandle {
    fn parse(line: &str) -> Option<Self> {
        let (handle, _) = line.split_once(':')?;
        let mut parts = handle.trim().split('/');
        let environment_id = parts.next()?.to_owned();
        let job_id = parts.next()?.to_owned();
        if parts.next().is_some() || environment_id.is_empty() || job_id.is_empty() {
            return None;
        }
        Some(Self {
            environment_id,
            job_id,
        })
    }

    fn json_arg(&self) -> Value {
        json!({
            "environment_id": self.environment_id,
            "job_id": self.job_id
        })
    }

    fn parse_await_payload(text: &str) -> Vec<Self> {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Vec::new();
        };
        value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|result| {
                let summary = result.get("output")?.get("summary")?;
                Some(Self {
                    environment_id: summary.get("namespace")?.as_str()?.to_owned(),
                    job_id: summary.get("jobId")?.as_str()?.to_owned(),
                })
            })
            .collect()
    }
}

fn parse_job_promises(text: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(promises) = value.get("promises").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut keyed = promises
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key, value)))
        .collect::<Vec<_>>();
    keyed.sort_by(|(left, _), (right, _)| left.cmp(right));
    keyed
        .into_iter()
        .map(|(_, value)| value.to_owned())
        .collect()
}

fn current_run_tool_result(request: &LlmGenerationRequest) -> Option<&engine::ContextEntry> {
    current_run_tool_results(request).into_iter().next()
}

fn current_run_tool_results(request: &LlmGenerationRequest) -> Vec<&engine::ContextEntry> {
    request
        .request
        .context
        .entries
        .iter()
        .rev()
        .filter(|entry| {
            matches!(
                (&entry.source, &entry.kind),
                (
                    ContextEntrySource::Tool { run_id, .. },
                    ContextEntryKind::ToolResult { .. }
                ) if *run_id == request.run_id
            )
        })
        .collect()
}

fn io_error(error: impl std::fmt::Display) -> CoreAgentIoError {
    CoreAgentIoError::Failed {
        message: error.to_string(),
    }
}

struct SpawnedBridge {
    child: Child,
}

impl SpawnedBridge {
    fn start(
        bridge_bin: &PathBuf,
        gateway_url: &str,
        provider_id: &str,
        root: &PathBuf,
    ) -> anyhow::Result<Self> {
        let child = Command::new(bridge_bin)
            .arg("--gateway-url")
            .arg(gateway_url)
            .arg("--provider-id")
            .arg(provider_id)
            .arg("--target-id")
            .arg("local")
            .arg("--listen")
            .arg("127.0.0.1:0")
            .arg("--cwd")
            .arg(root)
            .arg("--fs-root")
            .arg(root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                anyhow::anyhow!("spawn host-bridge binary {}: {error}", bridge_bin.display())
            })?;
        Ok(Self { child })
    }
}

impl Drop for SpawnedBridge {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn env_live_features() -> api::FeaturesConfig {
    api::FeaturesConfig {
        environments: Some(api::EnvironmentsFeature {
            version: api::CURRENT_FEATURE_VERSION,
            providers: None,
            selection_tools: false,
            jobs: false,
        }),
        vfs: Some(api::VfsFeature {
            version: api::CURRENT_FEATURE_VERSION,
            workspace_links: Vec::new(),
            tools: Some(api::VfsToolSurface::Edit),
            prompts: Some(api::VfsPromptsConfig::default()),
            skills: Some(api::VfsSkillsConfig::default()),
        }),
        ..api::FeaturesConfig::default()
    }
}

fn env_live_features_with_jobs() -> api::FeaturesConfig {
    let mut features = env_live_features();
    features
        .environments
        .as_mut()
        .expect("environment feature")
        .jobs = true;
    features
}

async fn wait_for_bridge_attach(
    api: &GatewayAgentApi,
    session_id: &engine::SessionId,
    provider_id: &str,
) -> anyhow::Result<api::AgentApiOutcome<api::SessionEnvironmentActivateResponse>> {
    let started = Instant::now();
    let mut last_error = None;
    loop {
        if started.elapsed() > Duration::from_secs(30) {
            anyhow::bail!(
                "timed out waiting to attach host bridge provider {provider_id}; last error: {}",
                last_error.unwrap_or_else(|| "none".to_owned())
            );
        }
        let instance = api
            .list_environments(EnvironmentListParams {
                provider_id: Some(provider_id.to_owned()),
                status: Some(EnvironmentTargetStatusView::Ready),
            })
            .await
            .ok()
            .and_then(|response| {
                response
                    .result
                    .environments
                    .into_iter()
                    .find(|instance| instance.provider_target_id == "local")
            });
        let Some(instance) = instance else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        match api
            .activate_session_environment(SessionEnvironmentActivateParams {
                session_id: session_id.as_str().to_owned(),
                environment_id: instance.environment_id,
            })
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                last_error = Some(error.to_string());
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

fn host_bridge_binary_path() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var("HOST_BRIDGE_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!("HOST_BRIDGE_BIN does not exist: {}", path.display());
    }

    let current_exe = std::env::current_exe()?;
    let target_dir = current_exe
        .parent()
        .and_then(|deps| deps.parent())
        .ok_or_else(|| anyhow::anyhow!("cannot infer target dir from {}", current_exe.display()))?;
    let binary = target_dir.join("host-bridge");
    if binary.exists() {
        return Ok(binary);
    }
    anyhow::bail!(
        "host-bridge binary not found at {}; run `cargo build -p host-bridge` or set HOST_BRIDGE_BIN",
        binary.display()
    );
}

struct FakeHostProvider {
    endpoint: String,
    state: Arc<FakeHostProviderState>,
    server: JoinHandle<()>,
}

impl FakeHostProvider {
    async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("ws://{}", listener.local_addr()?);
        let state = Arc::new(FakeHostProviderState::default());
        let server_state = state.clone();
        let server_endpoint = endpoint.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(handle_connection(
                    stream,
                    server_state.clone(),
                    server_endpoint.clone(),
                ));
            }
        });
        Ok(Self {
            endpoint,
            state,
            server,
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn controller_initialize_count(&self) -> usize {
        self.state
            .controller_initialize_count
            .load(Ordering::SeqCst)
    }

    fn attach_count(&self) -> usize {
        self.state.attach_count.load(Ordering::SeqCst)
    }

    fn create_count(&self) -> usize {
        self.state.create_count.load(Ordering::SeqCst)
    }

    fn close_count(&self) -> usize {
        self.state.close_count.load(Ordering::SeqCst)
    }

    fn reject_next_close(&self) {
        self.state.reject_next_close.store(true, Ordering::SeqCst);
    }

    fn process_start_count(&self) -> usize {
        self.state
            .process_starts
            .lock()
            .expect("process starts")
            .len()
    }

    fn process_cwds(&self) -> Vec<Option<String>> {
        self.state
            .process_starts
            .lock()
            .expect("process starts")
            .iter()
            .map(|params| params.cwd.as_ref().map(|cwd| cwd.as_str().to_owned()))
            .collect()
    }
}

impl Drop for FakeHostProvider {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[derive(Default)]
struct FakeHostProviderState {
    controller_initialize_count: AtomicUsize,
    list_targets_count: AtomicUsize,
    attach_count: AtomicUsize,
    create_count: AtomicUsize,
    close_count: AtomicUsize,
    reject_next_close: AtomicBool,
    process_starts: Mutex<Vec<StartProcessParams>>,
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: Arc<FakeHostProviderState>,
    endpoint: String,
) {
    let Ok(mut socket) = accept_async(stream).await else {
        return;
    };
    while let Some(message) = socket.next().await {
        let Ok(message) = message else {
            return;
        };
        let Ok(value) = websocket_json(message) else {
            continue;
        };
        let Some(id) = value.get("id").cloned() else {
            if value.get("method").and_then(Value::as_str) == Some(INITIALIZED_METHOD) {
                let _ = serde_json::from_value::<InitializedParams>(
                    value.get("params").cloned().unwrap_or(Value::Null),
                );
            }
            continue;
        };
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let response = match handle_request(method, params, state.as_ref(), &endpoint).await {
            Ok(result) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }),
            Err(message) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": "internal",
                    "message": message
                }
            }),
        };
        if socket
            .send(Message::Text(response.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
}

fn websocket_json(message: Message) -> anyhow::Result<Value> {
    match message {
        Message::Text(text) => Ok(serde_json::from_str(&text)?),
        Message::Binary(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Message::Close(_) => anyhow::bail!("websocket closed"),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
            anyhow::bail!("control frame")
        }
    }
}

async fn handle_request(
    method: &str,
    params: Value,
    state: &FakeHostProviderState,
    endpoint: &str,
) -> Result<Value, String> {
    match method {
        CONTROL_INITIALIZE_METHOD => {
            state
                .controller_initialize_count
                .fetch_add(1, Ordering::SeqCst);
            result_value(ControllerInitializeResponse {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                capabilities: ControllerCapabilities {
                    list_targets: true,
                    create_target: true,
                    attach_target: true,
                    get_target: true,
                    close_target: true,
                },
                implementation: ImplementationInfo {
                    name: "fake-host-provider".to_owned(),
                    version: Some("test".to_owned()),
                },
            })
        }
        LIST_TARGETS_METHOD => {
            state.list_targets_count.fetch_add(1, Ordering::SeqCst);
            result_value(ListTargetsResponse {
                targets: vec![target_summary(ATTACH_TARGET_ID)],
            })
        }
        ATTACH_TARGET_METHOD => {
            state.attach_count.fetch_add(1, Ordering::SeqCst);
            result_value(AttachTargetResponse {
                target: target_summary(ATTACH_TARGET_ID),
                connection: connection_spec(endpoint, ATTACH_TARGET_ID),
            })
        }
        CREATE_TARGET_METHOD => {
            state.create_count.fetch_add(1, Ordering::SeqCst);
            result_value(CreateTargetResponse {
                target: target_summary(CREATED_TARGET_ID),
                connection: connection_spec(endpoint, CREATED_TARGET_ID),
            })
        }
        CLOSE_TARGET_METHOD => {
            state.close_count.fetch_add(1, Ordering::SeqCst);
            if state.reject_next_close.swap(false, Ordering::SeqCst) {
                return Err("provider rejected close while target is occupied".to_owned());
            }
            result_value(CloseTargetResponse {
                target_id: HostTargetId::new(
                    params
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or(CREATED_TARGET_ID),
                ),
                status: HostTargetStatus::Closed,
            })
        }
        DATA_INITIALIZE_METHOD => result_value(InitializeResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            connection_id: HostConnectionId::new("fake-data-connection"),
            capabilities: host_capabilities(),
            default_cwd: Some("/workspace".to_owned()),
            implementation: ImplementationInfo {
                name: "fake-host-data".to_owned(),
                version: Some("test".to_owned()),
            },
        }),
        PROCESS_START_METHOD => {
            let params: StartProcessParams =
                serde_json::from_value(params).map_err(|error| error.to_string())?;
            let process_id = params.process_id.clone();
            state
                .process_starts
                .lock()
                .map_err(|error| error.to_string())?
                .push(params);
            result_value(StartProcessResponse { process_id })
        }
        PROCESS_READ_METHOD => result_value(ReadProcessResponse {
            chunks: vec![ProcessOutputChunk {
                seq: 1,
                stream: ProcessOutputStream::Stdout,
                chunk: ByteChunk::new(PROCESS_STDOUT.as_bytes()),
            }],
            next_seq: 2,
            exited: true,
            exit_code: Some(0),
            closed: true,
            failure: None,
            orphaned_descendants: false,
        }),
        other => Err(format!("unsupported fake host method: {other}")),
    }
}

fn result_value(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn target_summary(target_id: &str) -> HostTargetSummary {
    HostTargetSummary {
        target_id: HostTargetId::new(target_id),
        display_name: Some(target_id.to_owned()),
        status: HostTargetStatus::Ready,
        scope: HostScope::Default,
        capabilities: host_capabilities(),
        default_cwd: Some(HostPath::new("/workspace").expect("host cwd")),
        metadata: BTreeMap::new(),
    }
}

fn target_descriptor(endpoint: &str, target_id: &str) -> EnvironmentTargetDescriptorView {
    EnvironmentTargetDescriptorView {
        target: EnvironmentTargetSummaryView {
            target_id: target_id.to_owned(),
            status: EnvironmentTargetStatusView::Ready,
            scope: HostScopeView::Default,
            capabilities: host_capabilities_view(),
            display_name: Some(target_id.to_owned()),
            default_cwd: Some("/workspace".to_owned()),
            metadata: BTreeMap::new(),
        },
        connection: HostConnectionView {
            target_id: target_id.to_owned(),
            endpoint: endpoint.to_owned(),
            transport: HostTransportView::WebSocket,
            scope: HostScopeView::Default,
            default_cwd: Some("/workspace".to_owned()),
            capabilities: host_capabilities_view(),
        },
    }
}

fn host_capabilities_view() -> HostCapabilitiesView {
    HostCapabilitiesView {
        filesystem_read: true,
        filesystem_write: true,
        process_start: true,
        process_stdin: true,
        process_terminate: true,
        process_output_polling: true,
        process_output_notifications: false,
        process_pty: false,
        job_start: true,
        job_list: true,
        job_read: true,
        job_cancel: true,
        job_wait_hint: false,
        job_dependencies: true,
        job_queue_keys: true,
        network: true,
    }
}

fn connection_spec(endpoint: &str, target_id: &str) -> HostConnectionSpec {
    HostConnectionSpec {
        target_id: HostTargetId::new(target_id),
        endpoint: endpoint.to_owned(),
        transport: HostTransport::WebSocket,
        scope: HostScope::Default,
        default_cwd: Some(HostPath::new("/workspace").expect("host cwd")),
        capabilities: host_capabilities(),
    }
}

fn host_capabilities() -> HostCapabilities {
    HostCapabilities {
        filesystem_read: true,
        filesystem_write: true,
        filesystem_search: false,
        filesystem_glob: false,
        filesystem_ranged_read: false,
        process_start: true,
        process_stdin: true,
        process_terminate: true,
        process_output_polling: true,
        process_output_notifications: false,
        process_pty: false,
        job_start: true,
        job_list: true,
        job_read: true,
        job_cancel: true,
        job_wait_hint: false,
        job_dependencies: true,
        job_queue_keys: true,
        network: true,
    }
}
