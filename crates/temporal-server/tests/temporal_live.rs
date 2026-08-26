mod support;

use std::{env, future::Future, sync::Arc, time::Duration};

use api::{
    AgentApiErrorKind, AgentApiService, AgentProfileInput, ContextAppendEntry, ContextAppendParams,
    ContextAppendStatus, ContextEntryKindView, ContextMessageRoleView, FeaturesConfig,
    InitializeParams, InlineAgentProfile, InputItem, McpServerDeleteParams, McpServerInput,
    McpServerListParams, McpServerPutParams, McpServerReadParams, McpServerStatus,
    ProfileApplyParams, ProfileCreateParams, ProfileDeleteParams, ProfileDocument, ProfileId,
    ProfileInstructions, ProfileListParams, ProfilePutParams, ProfileReadParams, ProfileSource,
    RemoteMcpApprovalPolicy, RemoteMcpTransport, RunCancelParams, RunStartParams, RunStartSource,
    RunSteerParams, SessionConfig, SessionConfigPutParams, SessionDeleteParams,
    SessionEventsReadParams, SessionLifecycleStatus, SessionListParams, SessionReadParams,
    SessionStartParams, SessionStatus, TimersFeature,
};
use api_projection::model_to_api;
use async_trait::async_trait;
use engine::{
    ContextEntryInput, ContextEntryKind, ContextMessageRole, CoreAgentCommand, CoreAgentIoError,
    CoreAgentLlm, CoreAgentTools, LlmFinish, LlmGenerationFacts, LlmGenerationRequest,
    LlmGenerationResult, LlmGenerationStatus, ModelSelection, ObservedToolCall, SessionId,
    ToolCallId, ToolName,
    storage::{BlobStore, SessionStore},
};
use support::live::{
    LIVE_TEST_LOCK, fake_worker_activities, fake_worker_activities_for_run_control,
    fake_worker_activities_with_parallel_tool_calls, fake_worker_activities_with_tool_rounds,
    fake_worker_activities_with_transient_llm_failures, final_assistant_text, live_workflow_handle,
    openai_completions_live_model, openai_live_model, require_openai_live_env,
    require_storage_live_env, run_with_live_worker, wait_for_admission_failure,
    wait_for_session_status, wait_for_terminal_run,
};
use temporal_server::{
    DeploymentStores, UniverseRuntime, default_model_from_env,
    gateway::GatewayAgentApi,
    pg_store_from_env,
    subagents::AgentApiSubagentRuntime,
    worker::{ActivityState, SessionTools, WorkerActivities, core_runtime, worker_with_activities},
};
use temporal_workflow::{
    AgentAdmission, AgentAdmissionFailureKind, AgentSessionWorkflow, DEFAULT_TEMPORAL_NAMESPACE,
    DEFAULT_TEMPORAL_TARGET, LLM_RETRY_MAX_ATTEMPTS, connect_temporal,
};
use temporalio_client::{
    Client, WorkflowDescribeOptions, WorkflowQueryOptions, WorkflowSignalOptions,
    WorkflowTerminateOptions,
};
use tools::{
    concurrency::AWAIT_TOOL_NAME,
    subagents::{AGENT_RUN_TOOL_NAME, AGENT_SPAWN_TOOL_NAME},
};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_session_start_then_run_start_completes_fake_runs() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_fake_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_session_lifecycle_list_and_closed_only_delete() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_lifecycle_delete_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_continue_as_new_completes_later_fake_run() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_continue_as_new_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_hosted_run_exceeds_128_drive_transitions() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Thirty model/tool rounds reproduce the incident's transition count
    // shape while remaining well below the default history rollover threshold.
    let activities = fake_worker_activities_with_tool_rounds(30).await?;
    run_with_live_worker(activities, run_unbounded_hosted_run_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_run_start_missing_session_returns_not_found() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_missing_session_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_admission_failures_do_not_poison_workflow() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_admission_failure_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_context_append_is_idempotent_and_projected() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_context_append_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_mcp_and_session_links_materialize() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_mcp_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_profiles_create_start_and_apply_idempotently() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_profiles_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_profile_provisions_environment_for_session() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_profile_provision_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_environment_power_intent_converges_and_wakes_on_use() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let activities = fake_worker_activities().await?;
    run_with_live_worker(activities, run_environment_power_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_parallel_tool_batch_completes_per_call() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Three parallel-safe calls in one batch exercise the per-call activity
    // path: concurrent scheduling, completion-order resumes, and progressive
    // engine appends. One call fails terminally while its siblings succeed,
    // proving per-call independence. The terminal-run polling issues status
    // queries while the batch is in flight, which regresses the
    // workflow-waker class of bug (TMPRL1100) that batch-level activities
    // never hit.
    let activities = fake_worker_activities_with_parallel_tool_calls(3, Some(1)).await?;
    run_with_live_worker(activities, run_parallel_tool_batch_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_transient_llm_failures_retry_within_the_turn() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Two scripted transient provider failures precede a normal generation
    // (P116): the typed retryable activity failure makes Temporal retry the
    // pending `llm_generate` activity with durable backoff, honoring the
    // scripted suggested delay. The run must complete with one successful
    // generation and no transcript trace of the retried attempts.
    let activities = fake_worker_activities_with_transient_llm_failures(2).await?;
    run_with_live_worker(activities, run_transient_llm_retry_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_exhausted_llm_retries_fail_the_run_not_the_session() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Exactly the bounded attempt budget of transient failures (P116): the
    // first run exhausts `llm_generate`'s retry policy and fails at the
    // workflow boundary. The scripted budget is then consumed, so a second
    // run on the same session succeeds — proving exhaustion fails the run
    // while the session workflow survives for later runs.
    let attempts = usize::try_from(LLM_RETRY_MAX_ATTEMPTS).expect("attempt budget fits usize");
    let activities = fake_worker_activities_with_transient_llm_failures(attempts).await?;
    run_with_live_worker(activities, run_llm_retry_exhaustion_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_cancel_mid_generation_aborts_the_provider_call() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Cancel mid-generation: the fake provider sleeps inside generate. Cancelling the run
    // while that call is in flight must cancel the turn in the engine at
    // once (run reaches `cancelled`, no grace turn, no `failed`), abandon
    // the worker-side provider call through activity cancellation, and
    // leave the session serving a later run.
    let (activities, counters) =
        fake_worker_activities_for_run_control(Duration::from_secs(15), Duration::ZERO, 1).await?;
    run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_cancel_mid_generation_live_client(client, task_queue, session_id, counters)
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_cancel_during_tool_batch_records_cancelled_calls() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Cancel during a tool batch: the fake tool sleeps inside the call. Cancelling the
    // run while the batch executes records the call as cancelled with the
    // well-known content, drains the run to `cancelled`, and abandons the
    // worker-side tool execution.
    let (activities, counters) =
        fake_worker_activities_for_run_control(Duration::ZERO, Duration::from_secs(15), 1).await?;
    run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_cancel_during_tool_batch_live_client(client, task_queue, session_id, counters)
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_steering_lands_at_next_turn_boundary() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Steering: steer while the first (slow) generation is in flight. The
    // in-flight turn finishes untouched (its tool call runs), and the next
    // generation request carries the steering entry — the fake model echoes
    // it in the final answer. Steering a finished run is rejected.
    let (activities, counters) =
        fake_worker_activities_for_run_control(Duration::from_secs(4), Duration::ZERO, 1).await?;
    run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_steering_live_client(client, task_queue, session_id, counters)
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_steering_during_final_turn_adds_a_turn() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Steering while a single-turn run is generating its final answer: the
    // steering is materialized while the call is in flight, and instead of
    // completing on that final output the run gets one more turn whose
    // request carries the steering — so "steer" never silently does
    // nothing.
    let (activities, counters) =
        fake_worker_activities_for_run_control(Duration::from_secs(4), Duration::ZERO, 1).await?;
    run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_steering_final_turn_live_client(client, task_queue, session_id, counters)
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_queued_runs_return_promptly_and_run_in_order() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Queueing: runs started behind an active run return `queued` at
    // once, are visible in the session view, can be cancelled while queued,
    // and start in order after the active run ends. Cancelling the active
    // run leaves the queued one untouched and it runs next.
    let (activities, counters) =
        fake_worker_activities_for_run_control(Duration::from_secs(5), Duration::ZERO, 1).await?;
    run_with_live_worker(activities, move |client, task_queue, session_id| {
        run_queue_live_client(client, task_queue, session_id, counters)
    })
    .await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_run_returns_child_result_inline() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    run_with_scripted_subagent_live_worker(run_agent_run_inline_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_run_fans_out_three_children() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    run_with_scripted_subagent_live_worker(run_agent_run_fan_out_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_run_rejects_over_root_limit() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    run_with_scripted_subagent_live_worker(run_agent_run_limit_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_spawn_await_and_parent_cancel_closes_child() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // Cancel while parked on a spawned child: the await resolves cancelled,
    // the run-scoped promise cascades to the execution, and the execution
    // closes the child session.
    run_with_scripted_subagent_live_worker(run_agent_spawn_cancel_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_run_inherits_parent_environment() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // `environment: { type: "inherit" }` on the child profile activates the
    // parent's provisioned environment in the child and never closes it.
    run_with_scripted_subagent_live_worker(run_agent_run_inherit_environment_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra or compatible Temporal + Postgres env"]
async fn temporal_live_agent_run_deadline_fails_the_reply_and_closes_the_child()
-> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    // The grant's `deadlineMs` is enforced inside the execution: a child
    // that outlives it is closed and the parent's joined call fails with a
    // `deadline` envelope.
    run_with_scripted_subagent_live_worker(run_agent_run_deadline_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra, Postgres, Temporal, and OPENAI_API_KEY (costs real money)"]
async fn temporal_live_session_start_then_run_start_completes_openai_run() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;
    require_openai_live_env()?;

    let activities = WorkerActivities::from_env().await?;
    run_with_live_worker(activities, run_openai_live_client).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra, Postgres, Temporal, and OPENAI_API_KEY (costs real money)"]
async fn temporal_live_openai_completions_tool_call_round_trip() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;
    require_openai_live_env()?;

    let activities = WorkerActivities::from_env().await?;
    run_with_live_worker(activities, run_openai_completions_live_client).await
}

#[derive(Clone)]
struct SubagentScriptedLlm {
    blobs: Arc<dyn BlobStore>,
}

/// Scripted parent/child model for the sub-agent live scenarios. Parent
/// scripts are user messages: `AGENT_RUN <profile> [<count>]` emits that many
/// `agent_run` calls in one batch; `AGENT_RUN_SLOW <profile>` emits one
/// joined `agent_run` of the slow child; `AGENT_SPAWN_SLOW <profile>` emits one
/// `agent_spawn` and then awaits its promise. Children answer their briefs:
/// `CHILD_TASK <n>` completes at once, `SLOW_CHILD` completes after 12 s.
impl SubagentScriptedLlm {
    fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self { blobs }
    }

    async fn latest_text_for_kind(
        &self,
        request: &LlmGenerationRequest,
        matches_kind: impl Fn(&ContextEntryKind) -> bool,
    ) -> Result<Option<String>, CoreAgentIoError> {
        for entry in request.request.context.entries.iter().rev() {
            if matches_kind(&entry.kind) {
                return self
                    .blobs
                    .read_text(&entry.content_ref)
                    .await
                    .map(Some)
                    .map_err(io_error);
            }
        }
        Ok(None)
    }

    async fn tool_calls_result(
        &self,
        request: &LlmGenerationRequest,
        calls: Vec<(&str, serde_json::Value)>,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let mut context_entries = Vec::with_capacity(calls.len());
        let mut tool_calls = Vec::with_capacity(calls.len());
        for (index, (tool_name, arguments)) in calls.into_iter().enumerate() {
            if !request
                .request
                .tools
                .iter()
                .any(|tool| tool.name.as_str() == tool_name)
            {
                return Err(CoreAgentIoError::Failed {
                    message: format!("scripted subagent test expected {tool_name} to be available"),
                });
            }
            let arguments_ref = self
                .blobs
                .put_bytes(serde_json::to_vec(&arguments).map_err(io_error)?)
                .await
                .map_err(io_error)?;
            let call_id = ToolCallId::new(format!(
                "{tool_name}_call_{}_{index}",
                request.turn_id.as_u64()
            ));
            let tool_name = ToolName::new(tool_name);
            context_entries.push(ContextEntryInput {
                kind: ContextEntryKind::ToolCall {
                    call_id: call_id.clone(),
                    name: tool_name.clone(),
                },
                content_ref: arguments_ref.clone(),
                media_type: Some("application/json".to_owned()),
                preview: None,
                provider_kind: Some("subagent-script".to_owned()),
                provider_item_id: Some(call_id.as_str().to_owned()),
                token_estimate: None,
            });
            tool_calls.push(ObservedToolCall {
                call_id,
                tool_name,
                provider_kind: Some("subagent-script".to_owned()),
                arguments_ref,
                native_call_ref: None,
            });
        }
        Ok(LlmGenerationResult {
            run_id: request.run_id,
            turn_id: request.turn_id,
            status: LlmGenerationStatus::Succeeded,
            failure_ref: None,
            context_entries,
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("subagent-tools-{}", request.turn_id.as_u64())),
                finish: LlmFinish::ToolCalls,
                usage: None,
                tool_calls,
                context_token_estimate: None,
            },
        })
    }

    async fn final_result(
        &self,
        request: &LlmGenerationRequest,
        text: String,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
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
                preview: Some("subagent scripted final".to_owned()),
                provider_kind: Some("subagent-script".to_owned()),
                provider_item_id: None,
                token_estimate: None,
            }],
            facts: LlmGenerationFacts {
                provider_response_id: Some(format!("subagent-final-{}", request.turn_id.as_u64())),
                finish: LlmFinish::Stop,
                usage: None,
                tool_calls: Vec::new(),
                context_token_estimate: None,
            },
        })
    }
}

fn parse_agent_run_script(text: &str) -> Option<(&str, usize)> {
    let mut parts = text.split_whitespace();
    if parts.next()? != "AGENT_RUN" {
        return None;
    }
    let profile = parts.next()?;
    let count = parts
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(1);
    Some((profile, count))
}

fn parse_reply_promise(tool_result: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(tool_result).ok()?;
    // A reply-keyed acknowledgement shows the model one `promise` handle.
    value["promise"].as_str().map(ToOwned::to_owned)
}

#[async_trait]
impl CoreAgentLlm for SubagentScriptedLlm {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError> {
        let user_text = self
            .latest_text_for_kind(&request, |kind| {
                matches!(
                    kind,
                    ContextEntryKind::Message {
                        role: ContextMessageRole::User
                    }
                )
            })
            .await?
            .unwrap_or_default();

        if let Some(tool_result) = self
            .latest_text_for_kind(&request, |kind| {
                matches!(kind, ContextEntryKind::ToolResult { .. })
            })
            .await?
        {
            if user_text.starts_with("AGENT_SPAWN_SLOW")
                && let Some(promise_id) = parse_reply_promise(&tool_result)
            {
                return self
                    .tool_calls_result(
                        &request,
                        vec![(
                            AWAIT_TOOL_NAME,
                            serde_json::json!({
                                "promises": [promise_id],
                                "mode": "all",
                                "timeout_ms": 60_000
                            }),
                        )],
                    )
                    .await;
            }
            // Every tool result the parent sees ends its run, so the test
            // can read the sub-agent envelopes straight from the final text.
            let results = request
                .request
                .context
                .entries
                .iter()
                .filter(|entry| matches!(entry.kind, ContextEntryKind::ToolResult { .. }))
                .map(|entry| entry.content_ref.clone())
                .collect::<Vec<_>>();
            let mut texts = Vec::with_capacity(results.len());
            for content_ref in results {
                texts.push(self.blobs.read_text(&content_ref).await.map_err(io_error)?);
            }
            return self
                .final_result(&request, format!("parent done: {}", texts.join(" | ")))
                .await;
        }

        if let Some((profile, count)) = parse_agent_run_script(&user_text) {
            let calls = (0..count)
                .map(|index| {
                    (
                        AGENT_RUN_TOOL_NAME,
                        serde_json::json!({
                            "agent": profile,
                            "input": format!("CHILD_TASK {index}"),
                            "label": format!("child {index}")
                        }),
                    )
                })
                .collect();
            return self.tool_calls_result(&request, calls).await;
        }
        if let Some(profile) = user_text.strip_prefix("AGENT_RUN_SLOW ") {
            return self
                .tool_calls_result(
                    &request,
                    vec![(
                        AGENT_RUN_TOOL_NAME,
                        serde_json::json!({
                            "agent": profile.trim(),
                            "input": "SLOW_CHILD",
                            "label": "slow child"
                        }),
                    )],
                )
                .await;
        }
        if let Some(profile) = user_text.strip_prefix("AGENT_SPAWN_SLOW ") {
            return self
                .tool_calls_result(
                    &request,
                    vec![(
                        AGENT_SPAWN_TOOL_NAME,
                        serde_json::json!({
                            "agent": profile.trim(),
                            "input": "SLOW_CHILD",
                            "label": "slow child"
                        }),
                    )],
                )
                .await;
        }
        if let Some(index) = user_text.strip_prefix("CHILD_TASK ") {
            return self
                .final_result(&request, format!("child completed {}", index.trim()))
                .await;
        }
        if user_text.starts_with("SLOW_CHILD") {
            tokio::time::sleep(Duration::from_secs(12)).await;
            return self
                .final_result(&request, "slow child completed".to_owned())
                .await;
        }
        self.final_result(&request, "scripted run completed".to_owned())
            .await
    }
}

async fn run_with_scripted_subagent_live_worker<F, Fut>(run_client: F) -> anyhow::Result<()>
where
    F: FnOnce(
        Client,
        SessionId,
        Arc<GatewayAgentApi>,
        Arc<dyn BlobStore>,
        Arc<dyn SessionStore>,
        ModelSelection,
    ) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let task_queue = format!("lightspeed-agent-live-{}", uuid::Uuid::new_v4().simple());
    let session_id = SessionId::new(format!("session_live_{}", uuid::Uuid::new_v4().simple()));
    let temporal_target =
        env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| DEFAULT_TEMPORAL_TARGET.to_owned());
    let namespace =
        env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| DEFAULT_TEMPORAL_NAMESPACE.to_owned());

    let runtime = core_runtime()?;
    let client = connect_temporal(&temporal_target, &namespace).await?;
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = Arc::new(
        GatewayAgentApi::builder(client.clone(), store.clone())
            .with_task_queue(task_queue.clone())
            .with_default_model(model.clone())
            .build(),
    );

    let blobs_for_worker: Arc<dyn BlobStore> = store.clone();
    let llm = Arc::new(SubagentScriptedLlm::new(blobs_for_worker.clone())) as Arc<dyn CoreAgentLlm>;
    let hosted = Arc::new(SessionTools::from_pg_store(store.clone()));
    let tools = hosted.clone() as Arc<dyn CoreAgentTools>;
    let activities = WorkerActivities::for_universe(
        store.config().universe_id,
        ActivityState::from_pg_store(store.clone(), llm, tools)
            .with_hosted_tools(hosted)
            .with_workflow_tool_executions(client.clone())
            .with_subagent_runtime(Arc::new(AgentApiSubagentRuntime::new(api.clone()))),
    );
    let mut worker =
        worker_with_activities(&runtime, client.clone(), task_queue.clone(), activities)?;
    let shutdown_worker = worker.shutdown_handle();
    let worker_future = worker.run();
    tokio::pin!(worker_future);

    let blobs_for_client: Arc<dyn BlobStore> = store.clone();
    let sessions_for_client: Arc<dyn SessionStore> = store;
    let client_future = run_client(
        client.clone(),
        session_id,
        api,
        blobs_for_client,
        sessions_for_client,
        model,
    );
    tokio::pin!(client_future);

    let client_result = tokio::select! {
        worker_result = worker_future.as_mut() => {
            return match worker_result {
                Ok(()) => Err(anyhow::anyhow!("Temporal worker stopped before the live sub-agent test completed")),
                Err(error) => Err(error.context("Temporal worker failed")),
            };
        }
        client_result = client_future.as_mut() => client_result,
    };

    shutdown_worker();
    // A cancelled slow child may still be inside its scripted 12 s LLM
    // sleep; the worker drains that activity before it stops.
    tokio::time::timeout(Duration::from_secs(30), worker_future.as_mut())
        .await
        .map_err(|_| anyhow::anyhow!("Temporal worker did not shut down within 30 seconds"))??;
    client_result
}

fn io_error(error: impl std::fmt::Display) -> CoreAgentIoError {
    CoreAgentIoError::Failed {
        message: error.to_string(),
    }
}

/// `features.subagents` granting exactly one agent profile.
fn subagents_features(profile_id: &ProfileId, max_descendants: u32) -> api::FeaturesConfig {
    subagents_features_with_deadline(profile_id, max_descendants, 120_000)
}

fn subagents_features_with_deadline(
    profile_id: &ProfileId,
    max_descendants: u32,
    deadline_ms: u64,
) -> api::FeaturesConfig {
    api::FeaturesConfig {
        subagents: Some(api::SubagentsFeature {
            version: api::CURRENT_FEATURE_VERSION,
            agents: vec![api::SubagentAgentRef {
                profile_id: profile_id.clone(),
            }],
            max_depth: 2,
            max_descendants,
            max_concurrent: 4,
            deadline_ms,
        }),
        ..api::FeaturesConfig::default()
    }
}

/// A child profile for the scripted worker: no tools, scripted instructions.
async fn create_child_profile(api: &GatewayAgentApi) -> anyhow::Result<ProfileId> {
    let profile_id = ProfileId::new(format!(
        "live_subagent_child_{}",
        uuid::Uuid::new_v4().simple()
    ));
    api.create_profile(ProfileCreateParams {
        profile: AgentProfileInput {
            profile_id: profile_id.clone(),
            display_name: Some("Live child".to_owned()),
            description: Some("Answers CHILD_TASK briefs".to_owned()),
            document: ProfileDocument {
                config: Some(SessionConfig::default()),
                instructions: Some(ProfileInstructions::Text {
                    text: "You are a scripted live sub-agent.".to_owned(),
                }),
                environment: None,
            },
        },
    })
    .await?;
    Ok(profile_id)
}

async fn start_subagent_parent(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    model: &ModelSelection,
    profile_id: &ProfileId,
    max_descendants: u32,
    script: &str,
) -> anyhow::Result<String> {
    start_subagent_parent_with_features(
        api,
        session_id,
        model,
        subagents_features(profile_id, max_descendants),
        script,
    )
    .await
}

async fn start_subagent_parent_with_features(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    model: &ModelSelection,
    features: api::FeaturesConfig,
    script: &str,
) -> anyhow::Result<String> {
    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(model)),
            features: Some(features),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;
    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: script.to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    Ok(run.result.run.id)
}

async fn list_children(
    sessions: &Arc<dyn SessionStore>,
    parent: &SessionId,
) -> anyhow::Result<Vec<engine::storage::SessionRecord>> {
    Ok(sessions
        .list_sessions(engine::storage::ListSessions {
            cursor: None,
            limit: 50,
            root_session_id: None,
            parent_session_id: Some(parent.clone()),
        })
        .await?
        .sessions)
}

async fn wait_for_children_closed(
    sessions: &Arc<dyn SessionStore>,
    parent: &SessionId,
    expected: usize,
) -> anyhow::Result<Vec<engine::storage::SessionRecord>> {
    let mut children = Vec::new();
    wait_until(
        "sub-agent children to close",
        Duration::from_secs(60),
        async || {
            children = list_children(sessions, parent).await?;
            Ok(children.len() == expected
                && children.iter().all(|child| {
                    child.lifecycle_status == engine::storage::SessionLifecycleStatus::Closed
                }))
        },
    )
    .await?;
    Ok(children)
}

async fn cleanup_subagent_test(
    client: &Client,
    api: &GatewayAgentApi,
    profile_id: ProfileId,
    sessions: &[SessionId],
) {
    let _ = api.delete_profile(ProfileDeleteParams { profile_id }).await;
    for id in sessions {
        terminate_live_session(client, id, "subagent live test cleanup").await;
    }
}

async fn run_agent_run_inline_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    let profile_id = create_child_profile(api.as_ref()).await?;
    let profile = api
        .read_profile(ProfileReadParams {
            profile_id: profile_id.clone(),
        })
        .await?
        .result
        .profile;
    let run_id = start_subagent_parent(
        api.as_ref(),
        &session_id,
        &model,
        &profile_id,
        16,
        &format!("AGENT_RUN {profile_id}"),
    )
    .await?;

    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run_id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Completed);
    let parent_output = final_assistant_text(&parent_run).expect("parent assistant output");
    assert!(
        parent_output.contains("\"status\":\"completed\"")
            && parent_output.contains("child completed 0"),
        "expected the child's result envelope inline, got: {parent_output}"
    );
    let agent_calls = parent_run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AGENT_RUN_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(agent_calls.len(), 1, "expected one joined agent_run call");
    assert_eq!(agent_calls[0].status, api::ToolItemStatus::Succeeded);

    let children = wait_for_children_closed(&sessions, &session_id, 1).await?;
    let child = &children[0];
    let origin = child.origin.as_ref().expect("child origin");
    assert_eq!(origin.parent_session_id, session_id);
    assert_eq!(origin.root_session_id, session_id);
    assert_eq!(origin.depth, 1);
    assert_eq!(origin.profile_id, profile_id.as_str());
    assert_eq!(origin.profile_revision, profile.revision);
    assert_eq!(child.display_name.as_deref(), Some("child 0"));
    assert_eq!(child.source_session_id, None);

    let child_view = api
        .read_session(SessionReadParams {
            session_id: child.session_id.as_str().to_owned(),
        })
        .await?
        .result
        .session;
    let child_origin = child_view.origin.expect("child origin view");
    assert_eq!(child_origin.parent_session_id, session_id.as_str());
    assert_eq!(child_origin.parent_run_id, run_id);
    assert_eq!(child_origin.agent.profile_id, profile_id);
    assert_eq!(child_view.status, api::SessionStatus::Closed);
    let listed = api
        .list_sessions(api::SessionListParams {
            cursor: None,
            limit: None,
            root_session_id: Some(session_id.as_str().to_owned()),
            parent_session_id: None,
        })
        .await?;
    assert_eq!(listed.result.sessions.len(), 1);
    assert!(listed.result.sessions[0].origin.is_some());
    let parent_view = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(parent_view.result.session.origin.is_none());
    // The grant publishes the sub-agent catalog as a context entry (P134
    // slice 4); the model reads the menu from there, not from the schema.
    assert!(
        parent_view
            .result
            .session
            .active_context
            .entries
            .iter()
            .any(|entry| matches!(entry.kind, ContextEntryKindView::SubagentCatalog)),
        "expected a sub-agent catalog context entry on the parent"
    );

    let child_ids = children
        .iter()
        .map(|child| child.session_id.clone())
        .collect::<Vec<_>>();
    let mut all = vec![session_id];
    all.extend(child_ids);
    cleanup_subagent_test(&client, api.as_ref(), profile_id, &all).await;
    Ok(())
}

async fn run_agent_run_fan_out_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    let profile_id = create_child_profile(api.as_ref()).await?;
    let run_id = start_subagent_parent(
        api.as_ref(),
        &session_id,
        &model,
        &profile_id,
        16,
        &format!("AGENT_RUN {profile_id} 3"),
    )
    .await?;

    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run_id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Completed);
    let parent_output = final_assistant_text(&parent_run).expect("parent assistant output");
    for index in 0..3 {
        assert!(
            parent_output.contains(&format!("child completed {index}")),
            "expected child {index} result inline, got: {parent_output}"
        );
    }
    let agent_calls = parent_run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AGENT_RUN_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(
        agent_calls.len(),
        3,
        "expected three joined agent_run calls"
    );
    assert!(
        agent_calls
            .iter()
            .all(|call| call.status == api::ToolItemStatus::Succeeded)
    );
    let children = wait_for_children_closed(&sessions, &session_id, 3).await?;
    assert!(children.iter().all(|child| {
        child
            .origin
            .as_ref()
            .is_some_and(|origin| origin.depth == 1)
    }));

    let mut all = vec![session_id];
    all.extend(children.iter().map(|child| child.session_id.clone()));
    cleanup_subagent_test(&client, api.as_ref(), profile_id, &all).await;
    Ok(())
}

async fn run_agent_run_limit_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    let profile_id = create_child_profile(api.as_ref()).await?;
    // maxDescendants = 1: the second of two calls must be refused at the
    // reservation and surface as a failed call, not as a worker error.
    let run_id = start_subagent_parent(
        api.as_ref(),
        &session_id,
        &model,
        &profile_id,
        1,
        &format!("AGENT_RUN {profile_id} 2"),
    )
    .await?;

    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run_id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Completed);
    let agent_calls = parent_run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AGENT_RUN_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(agent_calls.len(), 2);
    let succeeded = agent_calls
        .iter()
        .filter(|call| call.status == api::ToolItemStatus::Succeeded)
        .count();
    let failed = agent_calls
        .iter()
        .filter(|call| call.status == api::ToolItemStatus::Failed)
        .count();
    assert_eq!((succeeded, failed), (1, 1), "one child, one refusal");
    let parent_output = final_assistant_text(&parent_run).expect("parent assistant output");
    assert!(
        parent_output.contains("maxDescendants"),
        "expected the refusal to name the limit, got: {parent_output}"
    );
    let children = wait_for_children_closed(&sessions, &session_id, 1).await?;

    let mut all = vec![session_id];
    all.extend(children.iter().map(|child| child.session_id.clone()));
    cleanup_subagent_test(&client, api.as_ref(), profile_id, &all).await;
    Ok(())
}

async fn run_agent_run_deadline_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    let profile_id = create_child_profile(api.as_ref()).await?;
    // The slow child takes 12 s; a 3 s grant deadline must cut it off.
    let run_id = start_subagent_parent_with_features(
        api.as_ref(),
        &session_id,
        &model,
        subagents_features_with_deadline(&profile_id, 16, 3_000),
        &format!("AGENT_RUN_SLOW {profile_id}"),
    )
    .await?;

    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run_id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Completed);
    let agent_calls = parent_run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AGENT_RUN_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(agent_calls.len(), 1, "expected one joined agent_run call");
    assert_eq!(
        agent_calls[0].status,
        api::ToolItemStatus::Failed,
        "a deadline resolves the joined call as failed"
    );
    let parent_output = final_assistant_text(&parent_run).expect("parent assistant output");
    assert!(
        parent_output.contains("\"status\":\"deadline\"")
            && !parent_output.contains("slow child completed"),
        "expected a deadline envelope without the child's output, got: {parent_output}"
    );

    // The execution closed the child well before its 12 s script finished.
    let children = wait_for_children_closed(&sessions, &session_id, 1).await?;
    assert_eq!(children[0].display_name.as_deref(), Some("slow child"));

    let mut all = vec![session_id];
    all.extend(children.iter().map(|child| child.session_id.clone()));
    cleanup_subagent_test(&client, api.as_ref(), profile_id, &all).await;
    Ok(())
}

async fn run_agent_run_inherit_environment_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    use std::collections::BTreeMap;

    use environment_protocol::shared::EnvironmentTransport;
    use environments::{
        EnvironmentConnectionSpec, EnvironmentProviderBindingId, EnvironmentProviderBindingStatus,
        EnvironmentProviderBindingStore, EnvironmentProviderId, EnvironmentProviderStore,
        PutEnvironmentProvider, PutEnvironmentProviderBinding,
    };

    let store = pg_store_from_env().await?;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let provider_id = format!("fake-inherit-{suffix}");
    let binding_id = format!("binding-inherit-{suffix}");
    store
        .put_provider(PutEnvironmentProvider {
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            display_name: Some("Live fake provider".to_owned()),
            controller_connection: EnvironmentConnectionSpec::new(
                "in-process",
                EnvironmentTransport::Provider {
                    provider_type: "fake".to_owned(),
                },
            ),
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;
    store
        .put_provider_binding(PutEnvironmentProviderBinding {
            universe_id: store.config().universe_id,
            binding_id: EnvironmentProviderBindingId::new(binding_id.clone()),
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            status: EnvironmentProviderBindingStatus::Enabled,
            expected_revision: None,
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;
    let environments_feature = api::EnvironmentsFeature {
        version: api::CURRENT_FEATURE_VERSION,
        providers: None,
        selection_tools: false,
        jobs: false,
    };

    // Child profile: inherits whatever environment its parent has active.
    let child_profile_id = ProfileId::new(format!("live_inherit_child_{suffix}"));
    api.create_profile(ProfileCreateParams {
        profile: AgentProfileInput {
            profile_id: child_profile_id.clone(),
            display_name: Some("Inheriting child".to_owned()),
            description: Some("Answers CHILD_TASK briefs on the parent\'s environment".to_owned()),
            document: ProfileDocument {
                config: Some(SessionConfig {
                    features: Some(api::FeaturesConfig {
                        environments: Some(environments_feature.clone()),
                        ..api::FeaturesConfig::default()
                    }),
                    ..SessionConfig::default()
                }),
                instructions: Some(ProfileInstructions::Text {
                    text: "You are a scripted live sub-agent.".to_owned(),
                }),
                environment: Some(api::ProfileEnvironment::Inherit {}),
            },
        },
    })
    .await?;

    // Parent: provisions its own environment and may run the child.
    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: None,
        profile: Some(ProfileSource::Inline {
            profile: Box::new(api::InlineAgentProfile {
                display_name: None,
                description: None,
                document: ProfileDocument {
                    config: Some(SessionConfig {
                        model: Some(model_to_api(&model)),
                        features: Some(api::FeaturesConfig {
                            environments: Some(environments_feature),
                            ..subagents_features(&child_profile_id, 16)
                        }),
                        ..SessionConfig::default()
                    }),
                    instructions: None,
                    environment: Some(api::ProfileEnvironment::Provision {
                        provider_id: provider_id.clone(),
                        template_id: "rust-v1".to_owned(),
                        display_name: None,
                        metadata: BTreeMap::new(),
                        retention: api::ProfileEnvironmentRetention::CloseWithSession,
                        idle_policy: None,
                        credentials: Vec::new(),
                    }),
                },
            }),
        }),
    })
    .await?;
    let parent_environment = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?
        .result
        .session
        .active_environment_id
        .expect("parent should have a provisioned environment");

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: format!("AGENT_RUN {child_profile_id}"),
                }],
            },
            config: None,
        })
        .await?;
    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run.result.run.id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Completed);
    let parent_output = final_assistant_text(&parent_run).expect("parent assistant output");
    assert!(
        parent_output.contains("child completed 0"),
        "expected the child result inline, got: {parent_output}"
    );

    let children = wait_for_children_closed(&sessions, &session_id, 1).await?;
    // The closed child no longer projects an active environment; its log
    // records the activation of exactly the parent's environment.
    let child_events = api
        .read_session_events(SessionEventsReadParams {
            session_id: children[0].session_id.as_str().to_owned(),
            after: None,
            limit: Some(500),
            wait_ms: Some(0),
        })
        .await?
        .result
        .events;
    let inherited = child_events.iter().any(|event| {
        serde_json::to_string(&event.kind).is_ok_and(|json| {
            json.contains("activeEnvironmentChanged") && json.contains(parent_environment.as_str())
        })
    });
    assert!(
        inherited,
        "child should have activated the parent\'s environment {parent_environment}; events: {}",
        serde_json::to_string(&child_events)?
    );
    // The child never closes an inherited environment.
    let environment = api
        .read_environment(api::EnvironmentReadParams {
            environment_id: parent_environment.clone(),
        })
        .await?
        .result
        .environment;
    assert!(
        !matches!(
            environment.status,
            api::EnvironmentLifecycleStatusView::Closing
                | api::EnvironmentLifecycleStatusView::Closed
        ),
        "inherited environment must stay open after the child closes, got {:?}",
        environment.status
    );

    let mut all = vec![session_id];
    all.extend(children.iter().map(|child| child.session_id.clone()));
    cleanup_subagent_test(&client, api.as_ref(), child_profile_id, &all).await;
    Ok(())
}

async fn run_agent_spawn_cancel_live_client(
    client: Client,
    session_id: SessionId,
    api: Arc<GatewayAgentApi>,
    _blobs: Arc<dyn BlobStore>,
    sessions: Arc<dyn SessionStore>,
    model: ModelSelection,
) -> anyhow::Result<()> {
    let profile_id = create_child_profile(api.as_ref()).await?;
    let run_id = start_subagent_parent(
        api.as_ref(),
        &session_id,
        &model,
        &profile_id,
        16,
        &format!("AGENT_SPAWN_SLOW {profile_id}"),
    )
    .await?;

    // The parent spawns, then parks on await; the slow child is running.
    let parked = wait_until(
        "the parent to park on await",
        Duration::from_secs(30),
        async || {
            let handle = live_workflow_handle(&client, &session_id)?;
            let status = handle
                .query(
                    AgentSessionWorkflow::status,
                    (),
                    WorkflowQueryOptions::default(),
                )
                .await?;
            Ok(status.active_waits == 1)
        },
    )
    .await;
    if let Err(error) = parked {
        let view = api
            .read_session(SessionReadParams {
                session_id: session_id.as_str().to_owned(),
            })
            .await?
            .result
            .session;
        let runs = serde_json::to_string_pretty(&view.runs)?;
        anyhow::bail!("{error}; parent runs: {runs}");
    }
    let mut children = Vec::new();
    wait_until(
        "the slow child to exist",
        Duration::from_secs(30),
        async || {
            children = list_children(&sessions, &session_id).await?;
            Ok(children.len() == 1)
        },
    )
    .await?;
    let child_id = children[0].session_id.clone();

    api.cancel_run(api::RunCancelParams {
        session_id: session_id.as_str().to_owned(),
        run_id: run_id.clone(),
    })
    .await?;
    let parent_run = wait_for_terminal_run(api.as_ref(), &session_id, &run_id).await?;
    assert_eq!(parent_run.status, api::RunStatus::Cancelled);
    let await_calls = parent_run
        .tool_batches
        .iter()
        .flat_map(|batch| &batch.calls)
        .filter(|call| call.tool_name == AWAIT_TOOL_NAME)
        .collect::<Vec<_>>();
    assert_eq!(await_calls.len(), 1, "expected one parked await call");

    // The cascade: run-scoped promise cancelled -> execution cancelled ->
    // child closed by the execution, well before its 12 s script finishes.
    let children = wait_for_children_closed(&sessions, &session_id, 1).await?;
    assert_eq!(children[0].session_id, child_id);

    cleanup_subagent_test(&client, api.as_ref(), profile_id, &[session_id, child_id]).await;
    Ok(())
}

async fn run_fake_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    let initialized = api.initialize(InitializeParams::default()).await?;
    assert_eq!(initialized.result.server_info.name, "lightspeed-agent");
    assert!(initialized.result.capabilities.history_read);
    assert!(initialized.result.capabilities.event_log);

    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(model_to_api(&model)),
                ..SessionConfig::default()
            }),
            profile: None,
        })
        .await?;
    assert_eq!(started.result.session.id, session_id.as_str());
    assert!(
        !started
            .result
            .session
            .active_context
            .entries
            .iter()
            .any(|entry| matches!(entry.kind, ContextEntryKindView::VfsCatalog))
    );

    let mut enabled_config = started
        .result
        .session
        .config
        .clone()
        .expect("started session config");
    let mut enabled_features = enabled_config.features.unwrap_or_default();
    enabled_features.vfs = Some(api::VfsFeature {
        version: api::CURRENT_FEATURE_VERSION,
        workspace_links: Vec::new(),
        tools: None,
        prompts: None,
        skills: None,
    });
    enabled_features.environments = Some(api::EnvironmentsFeature {
        version: api::CURRENT_FEATURE_VERSION,
        providers: None,
        selection_tools: false,
        jobs: false,
    });
    enabled_config.features = Some(enabled_features);
    let enabled = api
        .put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(started.result.session.config_revision),
            config: enabled_config,
        })
        .await?;
    assert!(
        enabled
            .result
            .session
            .active_context
            .entries
            .iter()
            .any(|entry| entry.kind == ContextEntryKindView::VfsCatalog)
    );
    let selection_tool_names = [
        tools::environment::control::ENVIRONMENT_LIST_TOOL_NAME,
        tools::environment::control::ENVIRONMENT_ACTIVATE_TOOL_NAME,
        tools::environment::control::ENVIRONMENT_DEACTIVATE_TOOL_NAME,
    ];
    assert!(
        enabled
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| {
                tool.tool_id == tools::environment::control::ENVIRONMENT_READ_TOOL_NAME
            })
    );
    assert!(selection_tool_names.iter().all(|name| {
        enabled
            .result
            .session
            .active_tools
            .tools
            .iter()
            .all(|tool| tool.tool_id != *name)
    }));

    let mut selection_config = enabled
        .result
        .session
        .config
        .clone()
        .expect("enabled session config");
    selection_config
        .features
        .as_mut()
        .and_then(|features| features.environments.as_mut())
        .expect("environment feature")
        .selection_tools = true;
    let selection_enabled = api
        .put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(enabled.result.session.config_revision),
            config: selection_config,
        })
        .await?;
    assert!(selection_tool_names.iter().all(|name| {
        selection_enabled
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| tool.tool_id == *name)
    }));
    assert!(
        selection_enabled
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| {
                tool.tool_id == tools::environment::control::ENVIRONMENT_READ_TOOL_NAME
            })
    );

    let mut disabled_config = selection_enabled
        .result
        .session
        .config
        .clone()
        .expect("enabled session config");
    if let Some(features) = disabled_config.features.as_mut() {
        features.vfs = None;
        features.environments = None;
    }
    let disabled = api
        .put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(selection_enabled.result.session.config_revision),
            config: disabled_config,
        })
        .await?;
    assert!(
        !disabled
            .result
            .session
            .active_context
            .entries
            .iter()
            .any(|entry| matches!(entry.kind, ContextEntryKindView::VfsCatalog))
    );

    let first = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "hello temporal agent".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let first_run = wait_for_terminal_run(&api, &session_id, &first.result.run.id).await?;
    let first_output = final_assistant_text(&first_run).expect("first assistant output");
    assert!(first_output.contains("Fake agent completed run"));

    let second = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: Some("live-retry-1".to_owned()),
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "second session-start input".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let second_run = wait_for_terminal_run(&api, &session_id, &second.result.run.id).await?;
    let second_output = final_assistant_text(&second_run).expect("second assistant output");
    assert!(second_output.contains("Fake agent completed run"));

    // Retried session/runs/start with the same submission id and input returns the
    // original run instead of starting a second one.
    let retried = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: Some("live-retry-1".to_owned()),
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "second session-start input".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    assert_eq!(retried.result.run.id, second.result.run.id);

    // Same submission id with different input is a typed rejection.
    let mismatch = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: Some("live-retry-1".to_owned()),
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "different input".to_owned(),
                }],
            },
            config: None,
        })
        .await;
    let mismatch_error = mismatch.expect_err("duplicate submission with different input fails");
    assert_eq!(mismatch_error.kind, api::AgentApiErrorKind::Rejected);

    // Retried session/start with the same session id returns the session.
    let restarted = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: None,
            profile: None,
        })
        .await?;
    assert_eq!(restarted.result.session.id, session_id.as_str());

    let read = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(read.result.session.runs.len() >= 2);

    let events = api
        .read_session_events(SessionEventsReadParams {
            wait_ms: Some(2_000),
            session_id: session_id.as_str().to_owned(),
            after: None,
            limit: Some(64),
        })
        .await?;
    assert!(!events.result.events.is_empty());

    // Long-poll at the head: no new events, so the read parks until the
    // wait elapses and returns an empty page with no cursor movement.
    let head_cursor = events.result.head_cursor;
    let parked_started = std::time::Instant::now();
    let parked = api
        .read_session_events(SessionEventsReadParams {
            wait_ms: Some(1_000),
            session_id: session_id.as_str().to_owned(),
            after: head_cursor,
            limit: Some(64),
        })
        .await?;
    assert!(parked.result.events.is_empty());
    assert!(parked.result.complete);
    assert!(parked_started.elapsed() >= std::time::Duration::from_millis(900));

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_lifecycle_delete_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client, store)
        .with_task_queue(task_queue)
        .with_default_model(model)
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: Some("Lifecycle delete live test".to_owned()),
        config: None,
        profile: None,
    })
    .await?;

    let listed = api.list_sessions(SessionListParams::default()).await?;
    let open_summary = listed
        .result
        .sessions
        .iter()
        .find(|summary| summary.id == session_id.as_str())
        .expect("started session is listed");
    assert_eq!(open_summary.lifecycle_status, SessionLifecycleStatus::Open);

    let delete_open = api
        .delete_session(SessionDeleteParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await
        .expect_err("open session deletion must be rejected");
    assert_eq!(delete_open.kind, AgentApiErrorKind::Rejected);

    api.close_session(api::SessionCloseParams {
        session_id: session_id.as_str().to_owned(),
        force: false,
    })
    .await?;

    let listed = api.list_sessions(SessionListParams::default()).await?;
    let closed_summary = listed
        .result
        .sessions
        .iter()
        .find(|summary| summary.id == session_id.as_str())
        .expect("closed session remains listed");
    assert_eq!(
        closed_summary.lifecycle_status,
        SessionLifecycleStatus::Closed
    );

    let deleted = api
        .delete_session(SessionDeleteParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert_eq!(
        deleted.result.session.lifecycle_status,
        SessionLifecycleStatus::Closed
    );

    let listed = api.list_sessions(SessionListParams::default()).await?;
    assert!(
        listed
            .result
            .sessions
            .iter()
            .all(|summary| summary.id != session_id.as_str())
    );
    let read_deleted = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await
        .expect_err("deleted session must not be readable");
    assert_eq!(read_deleted.kind, AgentApiErrorKind::NotFound);
    Ok(())
}

fn run_control_session_config(model: &ModelSelection) -> SessionConfig {
    // The VFS read-only tool surface gives the fake model a function tool to
    // call, so runs have a tool-call turn followed by a final turn.
    SessionConfig {
        model: Some(model_to_api(model)),
        features: Some(api::FeaturesConfig {
            vfs: Some(api::VfsFeature {
                version: api::CURRENT_FEATURE_VERSION,
                workspace_links: Vec::new(),
                tools: Some(api::VfsToolSurface::ReadOnly),
                prompts: None,
                skills: None,
            }),
            ..api::FeaturesConfig::default()
        }),
        ..SessionConfig::default()
    }
}

async fn run_control_api(
    client: &Client,
    task_queue: String,
    session_id: &SessionId,
    with_tools: bool,
) -> anyhow::Result<GatewayAgentApi> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let config = if with_tools {
        run_control_session_config(&model)
    } else {
        SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }
    };
    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(config),
        profile: None,
    })
    .await?;
    Ok(api)
}

async fn start_text_run(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    text: &str,
) -> anyhow::Result<api::RunView> {
    Ok(api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: text.to_owned(),
                }],
            },
            config: None,
        })
        .await?
        .result
        .run)
}

async fn read_run(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    run_id: &str,
) -> anyhow::Result<Option<api::RunView>> {
    let session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    Ok(session
        .result
        .session
        .runs
        .into_iter()
        .find(|run| run.id == run_id))
}

async fn wait_until(
    what: &str,
    timeout: Duration,
    mut check: impl AsyncFnMut() -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    loop {
        if check().await? {
            return Ok(());
        }
        if started.elapsed() > timeout {
            anyhow::bail!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn terminate_live_session(client: &Client, session_id: &SessionId, reason: &str) {
    if let Ok(handle) = live_workflow_handle(client, session_id) {
        let _ = handle
            .terminate(WorkflowTerminateOptions::builder().reason(reason).build())
            .await;
    }
}

async fn run_cancel_mid_generation_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    counters: temporal_server::worker::FakeRuntimeCounters,
) -> anyhow::Result<()> {
    let api = run_control_api(&client, task_queue, &session_id, false).await?;

    let run = start_text_run(&api, &session_id, "take your time").await?;
    assert_eq!(run.status, api::RunStatus::Running);
    wait_until(
        "the provider call to start",
        Duration::from_secs(10),
        async || Ok(counters.generations_started() >= 1),
    )
    .await?;

    let cancel_started = std::time::Instant::now();
    let cancelled = api
        .cancel_run(RunCancelParams {
            session_id: session_id.as_str().to_owned(),
            run_id: run.id.clone(),
        })
        .await?
        .result
        .run;
    assert!(
        matches!(
            cancelled.status,
            api::RunStatus::Cancelling | api::RunStatus::Cancelled
        ),
        "cancel must be acknowledged while the provider call is in flight, got {:?}",
        cancelled.status
    );
    assert!(
        cancel_started.elapsed() < Duration::from_secs(10),
        "cancel must not wait for the in-flight generation ({:?})",
        cancel_started.elapsed()
    );

    let terminal = wait_for_terminal_run(&api, &session_id, &run.id).await?;
    assert_eq!(terminal.status, api::RunStatus::Cancelled);
    assert!(
        cancel_started.elapsed() < Duration::from_secs(12),
        "the run must reach cancelled before the abandoned generation would have finished"
    );

    // The worker abandons the in-flight provider call once the activity
    // cancellation reaches it through the heartbeat; no second generation
    // (no grace turn) is ever requested for the cancelled run.
    wait_until(
        "the provider call to be abandoned",
        Duration::from_secs(20),
        async || Ok(counters.generations_abandoned() >= 1),
    )
    .await?;
    assert_eq!(counters.generations_started(), 1);
    assert_eq!(counters.generations_completed(), 0);

    let events = api
        .read_session_events(SessionEventsReadParams {
            session_id: session_id.as_str().to_owned(),
            after: None,
            limit: Some(500),
            wait_ms: None,
        })
        .await?
        .result
        .events;
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, api::SessionEventKindView::TurnCancelled { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, api::SessionEventKindView::RunFailed { .. }))
    );

    // The session keeps serving runs after a cancel.
    let next = start_text_run(&api, &session_id, "and again").await?;
    let next = wait_for_terminal_run_with_timeout(&api, &session_id, &next.id, 40).await?;
    assert_eq!(next.status, api::RunStatus::Completed);
    assert!(
        final_assistant_text(&next).is_some_and(|text| text.contains("Fake agent completed run"))
    );

    terminate_live_session(
        &client,
        &session_id,
        "cancel mid-generation live test cleanup",
    )
    .await;
    Ok(())
}

async fn run_cancel_during_tool_batch_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    counters: temporal_server::worker::FakeRuntimeCounters,
) -> anyhow::Result<()> {
    let api = run_control_api(&client, task_queue, &session_id, true).await?;

    let run = start_text_run(&api, &session_id, "call a slow tool").await?;
    wait_until(
        "the tool call to start",
        Duration::from_secs(15),
        async || Ok(counters.tool_calls_started() >= 1),
    )
    .await?;

    let cancel_started = std::time::Instant::now();
    let cancelled = api
        .cancel_run(RunCancelParams {
            session_id: session_id.as_str().to_owned(),
            run_id: run.id.clone(),
        })
        .await?
        .result
        .run;
    assert!(matches!(
        cancelled.status,
        api::RunStatus::Cancelling | api::RunStatus::Cancelled
    ));
    let terminal = wait_for_terminal_run(&api, &session_id, &run.id).await?;
    assert_eq!(terminal.status, api::RunStatus::Cancelled);
    assert!(
        cancel_started.elapsed() < Duration::from_secs(12),
        "the run must reach cancelled before the abandoned tool call would have finished"
    );

    // The cancelled call has a model-visible result so the conversation
    // stays well-formed for the next run.
    let cancelled_results = terminal
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                ContextEntryKindView::ToolResult { is_error: true, .. }
            ) && entry
                .text
                .as_deref()
                .is_some_and(|text| text.contains("tool call cancelled"))
        })
        .count();
    assert_eq!(cancelled_results, 1, "run entries: {:?}", terminal.entries);
    assert!(
        terminal.tool_batches.iter().any(|batch| {
            batch
                .calls
                .iter()
                .any(|call| call.status == api::ToolItemStatus::Cancelled)
        }),
        "tool batches: {:?}",
        terminal.tool_batches
    );

    wait_until(
        "the tool call to be abandoned",
        Duration::from_secs(20),
        async || Ok(counters.tool_calls_abandoned() >= 1),
    )
    .await?;
    // Exactly one generation (the tool-call turn); no second turn ran.
    assert_eq!(counters.generations_started(), 1);

    let next = start_text_run(&api, &session_id, "now answer normally").await?;
    let next = wait_for_terminal_run_with_timeout(&api, &session_id, &next.id, 40).await?;
    assert_eq!(next.status, api::RunStatus::Completed);

    terminate_live_session(
        &client,
        &session_id,
        "cancel during tool batch live test cleanup",
    )
    .await;
    Ok(())
}

async fn run_steering_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    counters: temporal_server::worker::FakeRuntimeCounters,
) -> anyhow::Result<()> {
    let api = run_control_api(&client, task_queue, &session_id, true).await?;

    let run = start_text_run(&api, &session_id, "do the task").await?;
    wait_until(
        "the first generation to start",
        Duration::from_secs(10),
        async || Ok(counters.generations_started() >= 1),
    )
    .await?;

    let steered = api
        .steer_run(RunSteerParams {
            session_id: session_id.as_str().to_owned(),
            run_id: run.id.clone(),
            items: vec![InputItem::Text {
                text: "also mention the moon".to_owned(),
            }],
        })
        .await?
        .result;
    assert_eq!(steered.steering_id, "steering_1");
    assert_eq!(steered.run.status, api::RunStatus::Running);
    // Admitted while the first generation was still in flight.
    assert_eq!(counters.generations_completed(), 0);

    let terminal = wait_for_terminal_run_with_timeout(&api, &session_id, &run.id, 40).await?;
    assert_eq!(terminal.status, api::RunStatus::Completed);
    let text = final_assistant_text(&terminal).expect("final answer");
    assert!(
        text.contains("Steering received: also mention the moon"),
        "final answer must reflect the steering delivered at the next turn: {text}"
    );
    // The in-flight turn finished untouched: its tool call ran, then the
    // final turn saw the steering. Two generations, one tool call.
    assert_eq!(counters.generations_started(), 2);
    assert_eq!(counters.generations_abandoned(), 0);
    assert_eq!(counters.tool_calls_completed(), 1);
    assert!(terminal.entries.iter().any(|entry| matches!(
        entry.source,
        Some(api::ContextEntrySourceView::Steering { .. })
    )));

    let late = api
        .steer_run(RunSteerParams {
            session_id: session_id.as_str().to_owned(),
            run_id: run.id.clone(),
            items: vec![InputItem::Text {
                text: "too late".to_owned(),
            }],
        })
        .await
        .expect_err("steering a finished run is rejected");
    assert_eq!(late.kind, AgentApiErrorKind::Rejected);

    terminate_live_session(&client, &session_id, "steering live test cleanup").await;
    Ok(())
}

async fn run_steering_final_turn_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    counters: temporal_server::worker::FakeRuntimeCounters,
) -> anyhow::Result<()> {
    // No tools: the fake model answers in one turn.
    let api = run_control_api(&client, task_queue, &session_id, false).await?;
    let run = start_text_run(&api, &session_id, "answer directly").await?;
    wait_until(
        "the generation to start",
        Duration::from_secs(10),
        async || Ok(counters.generations_started() >= 1),
    )
    .await?;
    let steered = api
        .steer_run(RunSteerParams {
            session_id: session_id.as_str().to_owned(),
            run_id: run.id.clone(),
            items: vec![InputItem::Text {
                text: "one more thing".to_owned(),
            }],
        })
        .await?
        .result;
    assert_eq!(steered.run.status, api::RunStatus::Running);

    let terminal = wait_for_terminal_run_with_timeout(&api, &session_id, &run.id, 40).await?;
    assert_eq!(terminal.status, api::RunStatus::Completed);
    assert_eq!(
        counters.generations_started(),
        2,
        "the run must take one more turn for the unconsumed steering"
    );
    let text = final_assistant_text(&terminal).expect("final answer");
    assert!(
        text.contains("Steering received: one more thing"),
        "the extra turn must carry the steering: {text}"
    );
    // The steering materializes after the in-flight turn: it sits between
    // the first answer and the extra turn's answer.
    let kinds = terminal
        .entries
        .iter()
        .map(|entry| match (&entry.kind, entry.source.as_ref()) {
            (
                ContextEntryKindView::Message {
                    role: ContextMessageRoleView::User,
                },
                Some(api::ContextEntrySourceView::Steering { .. }),
            ) => "steer",
            (
                ContextEntryKindView::Message {
                    role: ContextMessageRoleView::User,
                },
                _,
            ) => "user",
            (
                ContextEntryKindView::Message {
                    role: ContextMessageRoleView::Assistant,
                },
                _,
            ) => "assistant",
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["user", "assistant", "steer", "assistant"],
        "entries: {kinds:?}"
    );

    terminate_live_session(
        &client,
        &session_id,
        "steering final turn live test cleanup",
    )
    .await;
    Ok(())
}

async fn run_queue_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
    counters: temporal_server::worker::FakeRuntimeCounters,
) -> anyhow::Result<()> {
    let api = run_control_api(&client, task_queue, &session_id, false).await?;

    let first = start_text_run(&api, &session_id, "first").await?;
    assert_eq!(first.status, api::RunStatus::Running);

    let queued_at = std::time::Instant::now();
    let second = start_text_run(&api, &session_id, "second").await?;
    assert_eq!(second.status, api::RunStatus::Queued);
    assert!(
        queued_at.elapsed() < Duration::from_secs(3),
        "a queued run must be acknowledged without waiting for the active run ({:?})",
        queued_at.elapsed()
    );
    let third = start_text_run(&api, &session_id, "third").await?;
    assert_eq!(third.status, api::RunStatus::Queued);

    // Queued runs are part of the session view with their input.
    let queued_view = read_run(&api, &session_id, &second.id)
        .await?
        .expect("queued run in session view");
    assert_eq!(queued_view.status, api::RunStatus::Queued);
    assert!(matches!(
        &queued_view.source,
        api::RunViewSource::Input { items }
            if matches!(items.first(), Some(InputItem::Text { text }) if text == "second")
    ));

    // Steering a queued run is rejected; it has no turn yet.
    let steer_queued = api
        .steer_run(RunSteerParams {
            session_id: session_id.as_str().to_owned(),
            run_id: second.id.clone(),
            items: vec![InputItem::Text {
                text: "nope".to_owned(),
            }],
        })
        .await
        .expect_err("steering a queued run is rejected");
    assert_eq!(steer_queued.kind, AgentApiErrorKind::Rejected);

    // Cancel a queued run: dequeued as cancelled, nothing else changes.
    let cancelled_second = api
        .cancel_run(RunCancelParams {
            session_id: session_id.as_str().to_owned(),
            run_id: second.id.clone(),
        })
        .await?
        .result
        .run;
    assert_eq!(cancelled_second.status, api::RunStatus::Cancelled);
    assert!(
        read_run(&api, &session_id, &first.id)
            .await?
            .is_some_and(|run| run.status == api::RunStatus::Running)
    );

    // Cancel the active run while another is queued: only the active run is
    // cancelled, the queued one starts next and completes.
    let cancelled_first = api
        .cancel_run(RunCancelParams {
            session_id: session_id.as_str().to_owned(),
            run_id: first.id.clone(),
        })
        .await?
        .result
        .run;
    assert!(matches!(
        cancelled_first.status,
        api::RunStatus::Cancelling | api::RunStatus::Cancelled
    ));
    let first_terminal = wait_for_terminal_run(&api, &session_id, &first.id).await?;
    assert_eq!(first_terminal.status, api::RunStatus::Cancelled);
    let third_terminal =
        wait_for_terminal_run_with_timeout(&api, &session_id, &third.id, 40).await?;
    assert_eq!(third_terminal.status, api::RunStatus::Completed);
    assert!(
        final_assistant_text(&third_terminal).is_some_and(|text| text.contains(&format!(
            "completed run {}",
            third.id.trim_start_matches("run_")
        )))
    );

    // Another queued run on an idle session starts immediately.
    let fourth = start_text_run(&api, &session_id, "fourth").await?;
    assert_eq!(fourth.status, api::RunStatus::Running);
    let fourth_terminal =
        wait_for_terminal_run_with_timeout(&api, &session_id, &fourth.id, 40).await?;
    assert_eq!(fourth_terminal.status, api::RunStatus::Completed);

    // Exactly one generation per executed run (first, third, fourth); the
    // cancelled first run never got a second turn. Whether the worker
    // abandoned the first generation or it completed (and was discarded)
    // depends on heartbeat timing with this short delay and is covered by
    // the mid-generation cancel scenario.
    assert_eq!(counters.generations_started(), 3);
    assert_eq!(
        counters.generations_completed() + counters.generations_abandoned(),
        3
    );

    terminate_live_session(&client, &session_id, "queue live test cleanup").await;
    Ok(())
}

/// `wait_for_terminal_run` with a longer budget, for scenarios whose fake
/// provider sleeps on purpose.
async fn wait_for_terminal_run_with_timeout(
    api: &GatewayAgentApi,
    session_id: &SessionId,
    run_id: &str,
    timeout_secs: u64,
) -> anyhow::Result<api::RunView> {
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(timeout_secs) {
            anyhow::bail!("timed out waiting for run {run_id} to finish");
        }
        if let Some(run) = read_run(api, session_id, run_id).await?
            && matches!(
                run.status,
                api::RunStatus::Completed | api::RunStatus::Failed | api::RunStatus::Cancelled
            )
        {
            return Ok(run);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn run_continue_as_new_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .with_continue_as_new_history_threshold(1)
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    let initial_temporal_run_id = live_workflow_handle(&client, &session_id)?
        .describe(WorkflowDescribeOptions::default())
        .await?
        .run_id()
        .to_owned();

    let first = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "first run before continue as new".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let first_run_id = first.result.run.id.clone();
    let first_run = wait_for_terminal_run(&api, &session_id, &first_run_id).await?;
    assert_eq!(first_run.id, first_run_id);
    let continued_temporal_run_id = live_workflow_handle(&client, &session_id)?
        .describe(WorkflowDescribeOptions::default())
        .await?
        .run_id()
        .to_owned();
    assert_ne!(
        continued_temporal_run_id, initial_temporal_run_id,
        "the active Lightspeed run should cross a Temporal execution boundary"
    );

    let second = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "second run after continue as new".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let second_run = wait_for_terminal_run(&api, &session_id, &second.result.run.id).await?;
    let second_output = final_assistant_text(&second_run).expect("second assistant output");
    assert!(second_output.contains("Fake agent completed run"));

    let read = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert!(
        read.result.session.runs.len() >= 2,
        "projected session should include runs committed before and after continue-as-new"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent continue-as-new live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_parallel_tool_batch_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    // The VFS tool surface derives parallel-safe function tools (vfs reads),
    // so the fake model's three calls form one concurrent per-call group.
    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            features: Some(api::FeaturesConfig {
                vfs: Some(api::VfsFeature {
                    version: api::CURRENT_FEATURE_VERSION,
                    workspace_links: Vec::new(),
                    tools: Some(api::VfsToolSurface::ReadOnly),
                    prompts: None,
                    skills: None,
                }),
                ..api::FeaturesConfig::default()
            }),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    let started = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run a parallel tool batch".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &started.result.run.id).await?;
    assert_eq!(run.status, api::RunStatus::Completed);
    assert!(
        final_assistant_text(&run).is_some_and(|text| text.contains("Fake agent completed run"))
    );

    let mut results = run
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            ContextEntryKindView::ToolResult { call_id, is_error } => {
                Some((call_id.clone(), *is_error))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    results.sort();
    assert_eq!(
        results,
        vec![
            ("agent_call_1_0".to_owned(), false),
            ("agent_call_1_1".to_owned(), true),
            ("agent_call_1_2".to_owned(), false),
        ],
        "every call of the parallel batch must reach its own terminal result: \
         the scripted failure fails alone and its siblings succeed"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("parallel tool batch live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_transient_llm_retry_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model)
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: None,
        profile: None,
    })
    .await?;

    let started = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "retry through transient provider failures".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &started.result.run.id).await?;
    assert_eq!(
        run.status,
        api::RunStatus::Completed,
        "transient provider failures within the retry budget must not fail the run"
    );
    assert!(
        final_assistant_text(&run).is_some_and(|text| text.contains("Fake agent completed run"))
    );
    // Intermediate transient attempts are runtime facts, not session events:
    // the transcript holds exactly one successful assistant generation.
    let assistant_messages = run
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                ContextEntryKindView::Message {
                    role: ContextMessageRoleView::Assistant
                }
            )
        })
        .count();
    assert_eq!(assistant_messages, 1);
    assert!(
        run.entries.iter().all(|entry| !entry
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("scripted transient provider failure")),
        "transient attempts must not leak into model context"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("transient llm retry live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_llm_retry_exhaustion_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model)
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: None,
        profile: None,
    })
    .await?;

    let first = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "exhaust the provider retry budget".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let first_run = wait_for_terminal_run(&api, &session_id, &first.result.run.id).await?;
    assert_eq!(
        first_run.status,
        api::RunStatus::Failed,
        "exhausted provider retries must fail the run with a terminal generation result"
    );

    // The session workflow survived exhaustion, and the scripted transient
    // budget is consumed, so the next run on the same session succeeds.
    let second = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "recover after the provider outage".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let second_run = wait_for_terminal_run(&api, &session_id, &second.result.run.id).await?;
    assert_eq!(
        second_run.status,
        api::RunStatus::Completed,
        "a run after provider recovery must succeed on the surviving session workflow"
    );
    assert!(
        final_assistant_text(&second_run)
            .is_some_and(|text| text.contains("Fake agent completed run"))
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("llm retry exhaustion live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_unbounded_hosted_run_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            features: Some(api::FeaturesConfig {
                vfs: Some(api::VfsFeature {
                    version: api::CURRENT_FEATURE_VERSION,
                    workspace_links: Vec::new(),
                    tools: None,
                    prompts: None,
                    skills: None,
                }),
                ..api::FeaturesConfig::default()
            }),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;
    let initial_temporal_run_id = live_workflow_handle(&client, &session_id)?
        .describe(WorkflowDescribeOptions::default())
        .await?
        .run_id()
        .to_owned();

    let started = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "complete thirty verification tool rounds".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &started.result.run.id).await?;
    assert!(
        final_assistant_text(&run).is_some_and(|text| text.contains("Fake agent completed run"))
    );
    let final_temporal_run_id = live_workflow_handle(&client, &session_id)?
        .describe(WorkflowDescribeOptions::default())
        .await?
        .run_id()
        .to_owned();
    assert_eq!(
        final_temporal_run_id, initial_temporal_run_id,
        "step count alone must not continue as new"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("unbounded hosted run live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_missing_session_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client, store)
        .with_task_queue(task_queue)
        .with_default_model(model)
        .build();

    let error = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "this should not create a session".to_owned(),
                }],
            },
            config: None,
        })
        .await
        .expect_err("missing session session/runs/start should fail");
    assert!(matches!(error.kind, AgentApiErrorKind::NotFound));
    Ok(())
}

async fn run_context_append_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    let first_text = "[telegram:group Engineering] Alice (12:01): the deploy looks stuck";
    let second_text = "[telegram:group Engineering] Bob (12:02): restarting the worker now";
    let appended = api
        .append_context(ContextAppendParams {
            session_id: session_id.as_str().to_owned(),
            entries: vec![
                ContextAppendEntry {
                    key: "channel.room.msg-1".to_owned(),
                    item: InputItem::Text {
                        text: first_text.to_owned(),
                    },
                },
                ContextAppendEntry {
                    key: "channel.room.msg-2".to_owned(),
                    item: InputItem::Text {
                        text: second_text.to_owned(),
                    },
                },
            ],
        })
        .await?;
    assert_eq!(
        appended
            .result
            .results
            .iter()
            .map(|result| (result.key.as_str(), result.status))
            .collect::<Vec<_>>(),
        vec![
            ("channel.room.msg-1", ContextAppendStatus::Applied),
            ("channel.room.msg-2", ContextAppendStatus::Applied)
        ]
    );
    let first_revision = appended.result.context_revision;

    // Room events are visible as ordinary user-message context items.
    let read = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    let context_texts: Vec<&str> = read
        .result
        .session
        .active_context
        .entries
        .iter()
        .filter_map(|entry| match entry.kind {
            ContextEntryKindView::Message {
                role: ContextMessageRoleView::User,
            } => entry.text.as_deref(),
            _ => None,
        })
        .collect();
    assert!(context_texts.contains(&first_text));
    assert!(context_texts.contains(&second_text));

    // Re-sending the same batch is a no-op: keys are the idempotency handle.
    let replayed = api
        .append_context(ContextAppendParams {
            session_id: session_id.as_str().to_owned(),
            entries: vec![
                ContextAppendEntry {
                    key: "channel.room.msg-1".to_owned(),
                    item: InputItem::Text {
                        text: first_text.to_owned(),
                    },
                },
                ContextAppendEntry {
                    key: "channel.room.msg-2".to_owned(),
                    item: InputItem::Text {
                        text: second_text.to_owned(),
                    },
                },
            ],
        })
        .await?;
    assert_eq!(
        replayed
            .result
            .results
            .iter()
            .map(|result| (result.key.as_str(), result.status))
            .collect::<Vec<_>>(),
        vec![
            ("channel.room.msg-1", ContextAppendStatus::Unchanged),
            ("channel.room.msg-2", ContextAppendStatus::Unchanged)
        ]
    );
    assert_eq!(replayed.result.context_revision, first_revision);

    // Same key with different content upserts in place.
    let edited = api
        .append_context(ContextAppendParams {
            session_id: session_id.as_str().to_owned(),
            entries: vec![ContextAppendEntry {
                key: "channel.room.msg-2".to_owned(),
                item: InputItem::Text {
                    text: "[telegram:group Engineering] Bob (12:02): edited message".to_owned(),
                },
            }],
        })
        .await?;
    assert_eq!(
        edited
            .result
            .results
            .iter()
            .map(|result| (result.key.as_str(), result.status))
            .collect::<Vec<_>>(),
        vec![("channel.room.msg-2", ContextAppendStatus::Applied)]
    );
    assert!(edited.result.context_revision > first_revision);

    // Invalid input is rejected at admission with a typed error.
    let empty = api
        .append_context(ContextAppendParams {
            session_id: session_id.as_str().to_owned(),
            entries: Vec::new(),
        })
        .await;
    assert_eq!(
        empty.expect_err("empty append must fail").kind,
        AgentApiErrorKind::InvalidRequest
    );
    let blank_item = api
        .append_context(ContextAppendParams {
            session_id: session_id.as_str().to_owned(),
            entries: vec![ContextAppendEntry {
                key: "channel.room.msg-3".to_owned(),
                item: InputItem::Text {
                    text: "   ".to_owned(),
                },
            }],
        })
        .await;
    assert_eq!(
        blank_item.expect_err("blank item must fail").kind,
        AgentApiErrorKind::InvalidRequest
    );

    // A run started after the appends completes normally with the room
    // context present in the session.
    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "summarize the room".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    let output = final_assistant_text(&run).expect("assistant output");
    assert!(output.contains("Fake agent completed run"));

    Ok(())
}

async fn run_admission_failure_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    let handle = live_workflow_handle(&client, &session_id)?;
    handle
        .signal(
            AgentSessionWorkflow::submit_admissions,
            vec![AgentAdmission {
                // No run is active, so admission rejects this command; the
                // session must keep serving later admissions regardless.
                command: CoreAgentCommand::RequestRunSteering { input: Vec::new() },
                correlation_token: None,
            }],
            WorkflowSignalOptions::default(),
        )
        .await?;
    wait_for_admission_failure(
        &client,
        &session_id,
        AgentAdmissionFailureKind::RejectedCommand,
    )
    .await?;

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "valid run after malformed command".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    let output = final_assistant_text(&run).expect("assistant output");
    assert!(output.contains("Fake agent completed run"));

    handle
        .signal(
            AgentSessionWorkflow::submit_admissions,
            vec![AgentAdmission {
                command: CoreAgentCommand::CloseSession { force: false },
                correlation_token: None,
            }],
            WorkflowSignalOptions::default(),
        )
        .await?;
    wait_for_session_status(&api, &session_id, SessionStatus::Closed).await?;

    let error = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run after close should be rejected".to_owned(),
                }],
            },
            config: None,
        })
        .await
        .expect_err("closed session session/runs/start should be rejected");
    assert!(matches!(error.kind, AgentApiErrorKind::Rejected));
    let session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?;
    assert_eq!(session.result.session.status, SessionStatus::Closed);

    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent admission failure live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_mcp_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let server_id = format!("crm_{}", uuid::Uuid::new_v4().simple());

    let created = api
        .put_mcp_server(McpServerPutParams {
            server: McpServerInput {
                server_id: server_id.clone(),
                display_name: Some("CRM".to_owned()),
                server_url: format!("https://{server_id}.example.com/mcp"),
                transport: RemoteMcpTransport::Auto,
                default_server_label: "crm".to_owned(),
                description: Some("CRM MCP server".to_owned()),
                allowed_tools: Some(vec!["lookup_customer".to_owned()]),
                approval_default: RemoteMcpApprovalPolicy::Never,
                defer_loading_default: Some(true),
                auth_policy: api::McpServerAuthPolicy::None,
                credential: None,
                status: McpServerStatus::Active,
            },
            expected_revision: None,
        })
        .await?;
    assert_eq!(created.result.server.server_id, server_id);
    assert_eq!(created.result.server.revision, 1);

    let read = api
        .read_mcp_server(McpServerReadParams {
            server_id: server_id.clone(),
        })
        .await?;
    assert_eq!(read.result.server.default_server_label, "crm");

    let listed = api
        .list_mcp_servers(McpServerListParams {
            status: Some(McpServerStatus::Active),
        })
        .await?;
    assert!(
        listed
            .result
            .servers
            .iter()
            .any(|server| server.server_id == server_id)
    );

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }),
        profile: None,
    })
    .await?;

    // Link declaratively: put the session config back with the MCP server
    // declared in features.mcp, merged into the existing config document.
    let session = api
        .read_session(SessionReadParams {
            session_id: session_id.as_str().to_owned(),
        })
        .await?
        .result
        .session;
    let mut linked_config = session.config.clone().expect("session config");
    let mut features = linked_config.features.clone().unwrap_or_default();
    features.mcp = Some(api::McpFeature {
        version: api::CURRENT_FEATURE_VERSION,
        servers: vec![api::McpServerLink {
            server_id: server_id.clone(),
            allowed_tools: Some(vec!["lookup_customer".to_owned()]),
            approval: Some(RemoteMcpApprovalPolicy::Never),
            defer_loading: Some(true),
        }],
    });
    linked_config.features = Some(features);
    let linked = api
        .put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(session.config_revision),
            config: linked_config.clone(),
        })
        .await?;
    let tool_id = format!("mcp_{server_id}");
    assert!(
        linked
            .result
            .session
            .active_tools
            .tools
            .iter()
            .any(|tool| tool.tool_id == tool_id),
        "declared MCP tool should materialize into the session toolset"
    );

    let mcp_tools: Vec<_> = linked
        .result
        .session
        .active_tools
        .tools
        .iter()
        .filter(|tool| matches!(tool.kind, api::ToolKindView::RemoteMcp { .. }))
        .collect();
    assert_eq!(mcp_tools.len(), 1);
    let tool = mcp_tools[0];
    assert_eq!(tool.tool_id, tool_id);
    let api::ToolKindView::RemoteMcp {
        server_label,
        allowed_tools,
        approval,
        defer_loading,
        ..
    } = &tool.kind
    else {
        panic!("expected remote MCP tool kind");
    };
    assert_eq!(server_label, "crm");
    assert_eq!(allowed_tools, &Some(vec!["lookup_customer".to_owned()]));
    assert_eq!(*approval, RemoteMcpApprovalPolicy::Never);
    assert_eq!(*defer_loading, Some(true));

    // Unlink declaratively: put the config again without the server.
    let mut unlinked_config = linked_config;
    if let Some(features) = unlinked_config.features.as_mut() {
        features.mcp = None;
    }
    let unlinked = api
        .put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(linked.result.session.config_revision),
            config: unlinked_config,
        })
        .await?;
    assert!(
        unlinked
            .result
            .session
            .active_tools
            .tools
            .iter()
            .all(|tool| tool.tool_id != tool_id),
        "undeclared MCP tool should be removed from the session toolset"
    );
    assert!(
        unlinked
            .result
            .session
            .active_tools
            .tools
            .iter()
            .all(|tool| !matches!(tool.kind, api::ToolKindView::RemoteMcp { .. })),
        "no remote MCP tools should remain after undeclaring"
    );

    let deleted = api
        .delete_mcp_server(McpServerDeleteParams { server_id })
        .await?;
    assert_eq!(deleted.result.server.default_server_label, "crm");

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent MCP live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

/// P125: a `provision` profile creates one environment for the session it
/// starts, activates it while it is still provisioning, converges on retries
/// and repeated applies, and closes it with the session (or retains it).
async fn run_profile_provision_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    use std::collections::BTreeMap;

    use environment_protocol::shared::EnvironmentTransport;
    use environments::{
        EnvironmentConnectionSpec, EnvironmentProviderBindingId, EnvironmentProviderBindingStatus,
        EnvironmentProviderBindingStore, EnvironmentProviderId, EnvironmentProviderStore,
        PutEnvironmentProvider, PutEnvironmentProviderBinding,
    };

    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store.clone())
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let provider_id = format!("fake-profile-{suffix}");
    let binding_id = format!("binding-profile-{suffix}");
    let profile_id = ProfileId::new(format!("live_provision_{suffix}"));

    // Register the in-process fake provider and bind it to this universe
    // directly through the store: the operator API is deployment-scoped and
    // this test drives one universe's gateway.
    store
        .put_provider(PutEnvironmentProvider {
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            display_name: Some("Live fake provider".to_owned()),
            controller_connection: EnvironmentConnectionSpec::new(
                "in-process",
                EnvironmentTransport::Provider {
                    provider_type: "fake".to_owned(),
                },
            ),
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;
    store
        .put_provider_binding(PutEnvironmentProviderBinding {
            universe_id: store.config().universe_id,
            binding_id: EnvironmentProviderBindingId::new(binding_id.clone()),
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            status: EnvironmentProviderBindingStatus::Enabled,
            expected_revision: None,
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;

    let provision_document = |retention: api::ProfileEnvironmentRetention| ProfileDocument {
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            features: Some(api::FeaturesConfig {
                environments: Some(api::EnvironmentsFeature {
                    version: api::CURRENT_FEATURE_VERSION,
                    providers: None,
                    selection_tools: false,
                    jobs: false,
                }),
                ..api::FeaturesConfig::default()
            }),
            ..SessionConfig::default()
        }),
        instructions: None,
        environment: Some(api::ProfileEnvironment::Provision {
            provider_id: provider_id.clone(),
            template_id: "rust-v1".to_owned(),
            display_name: None,
            metadata: BTreeMap::from([("role".to_owned(), "sandbox".to_owned())]),
            retention,
            idle_policy: None,
            credentials: Vec::new(),
        }),
    };
    api.create_profile(ProfileCreateParams {
        profile: AgentProfileInput {
            profile_id: profile_id.clone(),
            display_name: Some("Live provisioning profile".to_owned()),
            description: None,
            document: provision_document(api::ProfileEnvironmentRetention::CloseWithSession),
        },
    })
    .await?;

    // A profile that provisions from an unknown provider fails before any
    // session exists.
    let rejected = api
        .start_session(SessionStartParams {
            session_id: Some(format!("{}_rejected", session_id.as_str())),
            display_name: None,
            config: None,
            profile: Some(ProfileSource::Inline {
                profile: Box::new(api::InlineAgentProfile {
                    display_name: None,
                    description: None,
                    document: ProfileDocument {
                        environment: Some(api::ProfileEnvironment::Provision {
                            provider_id: format!("missing-{suffix}"),
                            template_id: "rust-v1".to_owned(),
                            display_name: None,
                            metadata: BTreeMap::new(),
                            retention: api::ProfileEnvironmentRetention::CloseWithSession,
                            idle_policy: None,
                            credentials: Vec::new(),
                        }),
                        ..provision_document(api::ProfileEnvironmentRetention::CloseWithSession)
                    },
                }),
            }),
        })
        .await;
    assert!(
        rejected.is_err(),
        "unknown provider must be rejected before start"
    );
    assert!(
        api.read_session(api::SessionReadParams {
            session_id: format!("{}_rejected", session_id.as_str()),
        })
        .await
        .is_err(),
        "no session may exist after a pre-start rejection"
    );

    // Start: the environment is created and activated while still
    // provisioning (no reconciler has run yet).
    let start = |session_id: String| {
        api.start_session(SessionStartParams {
            session_id: Some(session_id),
            display_name: None,
            config: None,
            profile: Some(ProfileSource::Named {
                profile_id: profile_id.clone(),
            }),
        })
    };
    let started = start(session_id.as_str().to_owned()).await?;
    let active = started
        .result
        .session
        .active_environment_id
        .clone()
        .expect("profile provisioning activates the new environment");
    let listed = api
        .list_environments(api::EnvironmentListParams {
            origin_session_id: Some(session_id.as_str().to_owned()),
            ..api::EnvironmentListParams::default()
        })
        .await?
        .result
        .environments;
    assert_eq!(listed.len(), 1);
    let environment = &listed[0];
    assert_eq!(environment.environment_id, active);
    assert_eq!(
        environment.status,
        api::EnvironmentLifecycleStatusView::Provisioning
    );
    assert_eq!(
        environment.request_id,
        environments::EnvironmentProvisionRequestId::for_session(&session_id)
            .as_str()
            .to_owned()
    );
    let origin = environment
        .origin_session
        .as_ref()
        .expect("origin session provenance");
    assert_eq!(origin.session_id, session_id.as_str());
    assert_eq!(origin.profile_id.as_ref(), Some(&profile_id));
    assert!(origin.close_with_session);
    assert_eq!(
        environment.metadata.get("role").map(String::as_str),
        Some("sandbox")
    );

    // Retry the start and re-apply the profile: still exactly one environment.
    let restarted = start(session_id.as_str().to_owned()).await?;
    assert_eq!(
        restarted.result.session.active_environment_id.as_deref(),
        Some(active.as_str())
    );
    let applied = api
        .apply_profile(ProfileApplyParams {
            session_id: session_id.as_str().to_owned(),
            profile: ProfileSource::Named {
                profile_id: profile_id.clone(),
            },
            expected_config_revision: None,
            expected_tools_revision: None,
        })
        .await?;
    assert!(!applied.result.applied.environment_provisioned);
    assert!(!applied.result.applied.active_environment_changed);
    assert_eq!(
        api.list_environments(api::EnvironmentListParams {
            origin_session_id: Some(session_id.as_str().to_owned()),
            ..api::EnvironmentListParams::default()
        })
        .await?
        .result
        .environments
        .len(),
        1
    );

    // Drive the reconciler: the fake provider brings the environment to ready.
    wait_for_environment_status(&api, &active, api::EnvironmentLifecycleStatusView::Ready).await?;

    // Closing the session closes the environment (eager close, then the
    // reconciler finishes it).
    api.close_session(api::SessionCloseParams {
        session_id: session_id.as_str().to_owned(),
        force: false,
    })
    .await?;
    wait_for_environment_status(&api, &active, api::EnvironmentLifecycleStatusView::Closed).await?;

    // The sweep alone (no eager close) also converges: an environment whose
    // origin session is already closed is picked up by reconciliation.
    let swept = api
        .create_environment(api::EnvironmentCreateParams {
            request_id: format!("sweep-{suffix}"),
            binding_id: binding_id.clone(),
            template_id: "rust-v1".to_owned(),
            display_name: None,
            metadata: BTreeMap::new(),
            idle_policy: None,
        })
        .await?
        .result
        .environment;
    sqlx::query(
        "UPDATE environments SET origin_session_id = $3, origin_close_with_session = true \
         WHERE universe_id = $1 AND environment_id = $2",
    )
    .bind(store.config().universe_id)
    .bind(&swept.environment_id)
    .bind(session_id.as_str())
    .execute(store.pool())
    .await?;
    wait_for_environment_status(
        &api,
        &swept.environment_id,
        api::EnvironmentLifecycleStatusView::Closed,
    )
    .await?;

    // `retain`: the environment outlives its session.
    let retained_session = format!("{}_retain", session_id.as_str());
    let retained_start = api
        .start_session(SessionStartParams {
            session_id: Some(retained_session.clone()),
            display_name: None,
            config: None,
            profile: Some(ProfileSource::Inline {
                profile: Box::new(api::InlineAgentProfile {
                    display_name: None,
                    description: None,
                    document: provision_document(api::ProfileEnvironmentRetention::Retain),
                }),
            }),
        })
        .await?;
    let retained_environment = retained_start
        .result
        .session
        .active_environment_id
        .clone()
        .expect("retained environment activated");
    wait_for_environment_status(
        &api,
        &retained_environment,
        api::EnvironmentLifecycleStatusView::Ready,
    )
    .await?;
    api.close_session(api::SessionCloseParams {
        session_id: retained_session.clone(),
        force: false,
    })
    .await?;
    for _ in 0..5 {
        api.reconcile_environments_once().await?;
    }
    let retained = api
        .read_environment(api::EnvironmentReadParams {
            environment_id: retained_environment.clone(),
        })
        .await?
        .result
        .environment;
    assert_eq!(retained.status, api::EnvironmentLifecycleStatusView::Ready);
    assert!(
        !retained
            .origin_session
            .as_ref()
            .expect("origin")
            .close_with_session
    );
    api.close_environment(api::EnvironmentCloseParams {
        environment_id: retained_environment.clone(),
    })
    .await?;
    wait_for_environment_status(
        &api,
        &retained_environment,
        api::EnvironmentLifecycleStatusView::Closed,
    )
    .await?;

    let _ = api
        .delete_profile(api::ProfileDeleteParams {
            profile_id: profile_id.clone(),
        })
        .await;
    // Every environment above is closed, so the binding and provider can go.
    let _ = store
        .delete_provider_binding(
            store.config().universe_id,
            &EnvironmentProviderBindingId::new(binding_id.clone()),
        )
        .await;
    let _ = store
        .delete_provider(&EnvironmentProviderId::new(provider_id.clone()))
        .await;
    Ok(())
}

/// P126: power intent is recorded through the API, converged by the lifecycle
/// reconciler against the provider, and a powered-down environment wakes
/// transparently when a session selects it. Idle policy round-trips and the
/// power reaper treats an environment without a reachable daemon as
/// untouchable.
async fn run_environment_power_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    use std::collections::BTreeMap;

    use environment_protocol::shared::EnvironmentTransport;
    use environments::{
        EnvironmentConnectionSpec, EnvironmentProviderBindingId, EnvironmentProviderBindingStatus,
        EnvironmentProviderBindingStore, EnvironmentProviderId, EnvironmentProviderStore,
        PutEnvironmentProvider, PutEnvironmentProviderBinding,
    };

    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store.clone())
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let provider_id = format!("fake-power-{suffix}");
    let binding_id = format!("binding-power-{suffix}");
    store
        .put_provider(PutEnvironmentProvider {
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            display_name: Some("Live fake provider (power)".to_owned()),
            controller_connection: EnvironmentConnectionSpec::new(
                "in-process",
                EnvironmentTransport::Provider {
                    provider_type: "fake".to_owned(),
                },
            ),
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;
    store
        .put_provider_binding(PutEnvironmentProviderBinding {
            universe_id: store.config().universe_id,
            binding_id: EnvironmentProviderBindingId::new(binding_id.clone()),
            provider_id: EnvironmentProviderId::new(provider_id.clone()),
            status: EnvironmentProviderBindingStatus::Enabled,
            expected_revision: None,
            metadata: BTreeMap::new(),
            updated_at_ms: 1,
        })
        .await?;

    let policy = api::EnvironmentIdlePolicyView {
        pause_after_ms: Some(60_000),
        suspend_after_ms: None,
        stop_after_ms: Some(3_600_000),
        close_after_ms: None,
    };
    let created = api
        .create_environment(api::EnvironmentCreateParams {
            request_id: format!("power-{suffix}"),
            binding_id: binding_id.clone(),
            template_id: "rust-v1".to_owned(),
            display_name: Some("Power VM".to_owned()),
            metadata: BTreeMap::new(),
            idle_policy: Some(policy.clone()),
        })
        .await?
        .result
        .environment;
    let environment_id = created.environment_id.clone();
    assert_eq!(
        created.desired_power,
        api::EnvironmentPowerStateView::Running
    );
    assert_eq!(created.idle_policy.as_ref(), Some(&policy));
    assert!(created.incarnation.power_states.is_empty());

    // Before the provider reported power support, power changes are refused.
    let premature = api
        .put_environment_power(api::EnvironmentPowerPutParams {
            environment_id: environment_id.clone(),
            power: api::EnvironmentPowerStateView::Paused,
        })
        .await
        .expect_err("power change before observation is rejected");
    assert_eq!(premature.kind, api::AgentApiErrorKind::Rejected);

    let ready = wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Ready,
    )
    .await?;
    assert_eq!(
        ready.incarnation.power_states,
        vec![
            api::EnvironmentPowerStateView::Running,
            api::EnvironmentPowerStateView::Paused,
            api::EnvironmentPowerStateView::Suspended,
            api::EnvironmentPowerStateView::Stopped,
        ]
    );

    // A malformed idle policy is rejected; a valid replacement and a clear
    // round-trip.
    let bad_policy = api
        .put_environment_idle_policy(api::EnvironmentIdlePolicyPutParams {
            environment_id: environment_id.clone(),
            idle_policy: Some(api::EnvironmentIdlePolicyView {
                pause_after_ms: Some(10),
                stop_after_ms: Some(5),
                ..api::EnvironmentIdlePolicyView::default()
            }),
        })
        .await
        .expect_err("non-monotone idle policy is rejected");
    assert_eq!(bad_policy.kind, api::AgentApiErrorKind::InvalidRequest);
    let cleared = api
        .put_environment_idle_policy(api::EnvironmentIdlePolicyPutParams {
            environment_id: environment_id.clone(),
            idle_policy: None,
        })
        .await?
        .result
        .environment;
    assert!(cleared.idle_policy.is_none());
    let restored = api
        .put_environment_idle_policy(api::EnvironmentIdlePolicyPutParams {
            environment_id: environment_id.clone(),
            idle_policy: Some(policy.clone()),
        })
        .await?
        .result
        .environment;
    assert_eq!(restored.idle_policy.as_ref(), Some(&policy));

    // The reaper sees the candidate but cannot reach a daemon through the
    // fake provider, so it leaves the environment alone.
    let stats = api.reap_idle_environments_once().await?;
    assert_eq!(stats.candidates, 1);
    assert_eq!(stats.unreachable, 1);
    assert_eq!(stats.powered_down, 0);
    assert_eq!(stats.closed, 0);

    // Pause intent: recorded immediately, converged by the reconciler.
    let paused_intent = api
        .put_environment_power(api::EnvironmentPowerPutParams {
            environment_id: environment_id.clone(),
            power: api::EnvironmentPowerStateView::Paused,
        })
        .await?
        .result
        .environment;
    assert_eq!(
        paused_intent.desired_power,
        api::EnvironmentPowerStateView::Paused
    );
    assert_eq!(
        paused_intent.status,
        api::EnvironmentLifecycleStatusView::Ready
    );
    let paused = wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Paused,
    )
    .await?;
    assert_eq!(paused.desired_power, api::EnvironmentPowerStateView::Paused);
    // Paused is a filterable lifecycle status.
    assert!(
        api.list_environments(api::EnvironmentListParams {
            status: Some(api::EnvironmentLifecycleStatusView::Paused),
            ..api::EnvironmentListParams::default()
        })
        .await?
        .result
        .environments
        .iter()
        .any(|environment| environment.environment_id == environment_id)
    );

    // Wake-on-use: activating the paused environment for a session admits it
    // as intent and flips desired power back to running; the reconciler then
    // brings it to ready.
    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(model_to_api(&model)),
                features: Some(api::FeaturesConfig {
                    environments: Some(api::EnvironmentsFeature {
                        version: api::CURRENT_FEATURE_VERSION,
                        providers: None,
                        selection_tools: false,
                        jobs: false,
                    }),
                    ..api::FeaturesConfig::default()
                }),
                ..SessionConfig::default()
            }),
            profile: None,
        })
        .await?;
    assert!(started.result.session.active_environment_id.is_none());
    let activated = api
        .activate_session_environment(api::SessionEnvironmentActivateParams {
            session_id: session_id.as_str().to_owned(),
            environment_id: environment_id.clone(),
        })
        .await?;
    assert_eq!(
        activated.result.session.active_environment_id.as_deref(),
        Some(environment_id.as_str())
    );
    let woken = api
        .read_environment(api::EnvironmentReadParams {
            environment_id: environment_id.clone(),
        })
        .await?
        .result
        .environment;
    assert_eq!(woken.desired_power, api::EnvironmentPowerStateView::Running);
    wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Ready,
    )
    .await?;

    // Suspend and stop are ordinary intents on a provider that supports them.
    api.put_environment_power(api::EnvironmentPowerPutParams {
        environment_id: environment_id.clone(),
        power: api::EnvironmentPowerStateView::Suspended,
    })
    .await?;
    wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Suspended,
    )
    .await?;
    // Wake-on-use through the jobs API: creating a job against the
    // suspended environment fails typed `environment_not_ready` (never a
    // generic rejection) and flips desired power back to running — the
    // retry-with-backoff contract polling automations lean on.
    let job_not_ready = api
        .create_environment_jobs(api::EnvironmentJobCreateParams {
            environment_id: environment_id.clone(),
            request_id: format!("wake-probe-{suffix}"),
            jobs: vec![api::SessionJobStartSpecInput {
                name: Some("wake-probe".to_owned()),
                job_id: None,
                argv: vec!["true".to_owned()],
                cwd: None,
                env: BTreeMap::new(),
                stdin: None,
                timeout_ms: None,
                depends_on: Vec::new(),
                dependency_policy: api::SessionJobDependencyPolicyView::default(),
                queue_key: None,
            }],
        })
        .await
        .expect_err("jobs/create against a suspended environment is not ready");
    assert_eq!(
        job_not_ready.kind,
        api::AgentApiErrorKind::EnvironmentNotReady
    );
    let waking = api
        .read_environment(api::EnvironmentReadParams {
            environment_id: environment_id.clone(),
        })
        .await?
        .result
        .environment;
    assert_eq!(
        waking.desired_power,
        api::EnvironmentPowerStateView::Running
    );

    api.put_environment_power(api::EnvironmentPowerPutParams {
        environment_id: environment_id.clone(),
        power: api::EnvironmentPowerStateView::Stopped,
    })
    .await?;
    wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Offline,
    )
    .await?;
    api.put_environment_power(api::EnvironmentPowerPutParams {
        environment_id: environment_id.clone(),
        power: api::EnvironmentPowerStateView::Running,
    })
    .await?;
    wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Ready,
    )
    .await?;

    // External environments have no power control.
    let external = api
        .create_external_environment(api::EnvironmentExternalCreateParams {
            request_id: format!("power-external-{suffix}"),
            connection: api::EnvironmentConnectionView {
                endpoint: format!("ws://127.0.0.1:1/{suffix}"),
                transport: api::EnvironmentConnectionTransportView::WebSocket,
            },
            display_name: None,
            metadata: BTreeMap::new(),
        })
        .await?
        .result
        .environment;
    let external_rejected = api
        .put_environment_power(api::EnvironmentPowerPutParams {
            environment_id: external.environment_id.clone(),
            power: api::EnvironmentPowerStateView::Paused,
        })
        .await
        .expect_err("external environments have no power control");
    assert_eq!(external_rejected.kind, api::AgentApiErrorKind::Rejected);
    let external_policy_rejected = api
        .put_environment_idle_policy(api::EnvironmentIdlePolicyPutParams {
            environment_id: external.environment_id.clone(),
            idle_policy: Some(policy.clone()),
        })
        .await
        .expect_err("external environments have no idle policy");
    assert_eq!(
        external_policy_rejected.kind,
        api::AgentApiErrorKind::InvalidRequest
    );

    // Closing wins over power intent.
    api.close_session(api::SessionCloseParams {
        session_id: session_id.as_str().to_owned(),
        force: false,
    })
    .await?;
    api.close_environment(api::EnvironmentCloseParams {
        environment_id: environment_id.clone(),
    })
    .await?;
    wait_for_environment_status(
        &api,
        &environment_id,
        api::EnvironmentLifecycleStatusView::Closed,
    )
    .await?;
    let closed_rejected = api
        .put_environment_power(api::EnvironmentPowerPutParams {
            environment_id: environment_id.clone(),
            power: api::EnvironmentPowerStateView::Running,
        })
        .await
        .expect_err("closed environments cannot change power");
    assert_eq!(closed_rejected.kind, api::AgentApiErrorKind::InvalidRequest);
    api.close_environment(api::EnvironmentCloseParams {
        environment_id: external.environment_id.clone(),
    })
    .await?;
    wait_for_environment_status(
        &api,
        &external.environment_id,
        api::EnvironmentLifecycleStatusView::Closed,
    )
    .await?;

    let _ = store
        .delete_provider_binding(
            store.config().universe_id,
            &EnvironmentProviderBindingId::new(binding_id.clone()),
        )
        .await;
    let _ = store
        .delete_provider(&EnvironmentProviderId::new(provider_id.clone()))
        .await;
    Ok(())
}

async fn wait_for_environment_status(
    api: &GatewayAgentApi,
    environment_id: &str,
    expected: api::EnvironmentLifecycleStatusView,
) -> anyhow::Result<api::EnvironmentView> {
    let started = std::time::Instant::now();
    loop {
        api.reconcile_environments_once().await?;
        let environment = api
            .read_environment(api::EnvironmentReadParams {
                environment_id: environment_id.to_owned(),
            })
            .await?
            .result
            .environment;
        if environment.status == expected {
            return Ok(environment);
        }
        if started.elapsed() > Duration::from_secs(20) {
            anyhow::bail!(
                "timed out waiting for environment {environment_id} to reach {expected:?}; current status is {:?}",
                environment.status
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn run_profiles_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = default_model_from_env();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();
    let profile_id = ProfileId::new(format!("live_profile_{}", uuid::Uuid::new_v4().simple()));
    let server_id = format!("profile_crm_{}", uuid::Uuid::new_v4().simple());

    api.put_mcp_server(McpServerPutParams {
        server: McpServerInput {
            server_id: server_id.clone(),
            display_name: Some("Profile CRM".to_owned()),
            server_url: format!("https://{server_id}.example.com/mcp"),
            transport: RemoteMcpTransport::Auto,
            default_server_label: "profile_crm".to_owned(),
            description: Some("Profile live MCP server".to_owned()),
            allowed_tools: Some(vec!["lookup_customer".to_owned()]),
            approval_default: RemoteMcpApprovalPolicy::Never,
            defer_loading_default: Some(true),
            auth_policy: api::McpServerAuthPolicy::None,
            credential: None,
            status: McpServerStatus::Active,
        },
        expected_revision: None,
    })
    .await?;

    let created = api
        .create_profile(ProfileCreateParams {
            profile: AgentProfileInput {
                profile_id: profile_id.clone(),
                display_name: Some("Live profile".to_owned()),
                description: Some("Initial live profile".to_owned()),
                document: ProfileDocument {
                    config: Some(SessionConfig {
                        features: Some(api::FeaturesConfig {
                            mcp: Some(api::McpFeature {
                                version: api::CURRENT_FEATURE_VERSION,
                                servers: vec![api::McpServerLink {
                                    server_id: server_id.clone(),
                                    allowed_tools: Some(vec!["lookup_customer".to_owned()]),
                                    approval: Some(RemoteMcpApprovalPolicy::Never),
                                    defer_loading: Some(true),
                                }],
                            }),
                            timers: Some(api::TimersFeature {
                                version: api::CURRENT_FEATURE_VERSION,
                            }),
                            ..api::FeaturesConfig::default()
                        }),
                        ..SessionConfig::default()
                    }),
                    instructions: Some(ProfileInstructions::Text {
                        text: "Use the profile instructions in this live test.".to_owned(),
                    }),
                    environment: None,
                },
            },
        })
        .await?;
    assert_eq!(created.result.profile.profile_id, profile_id);
    assert_eq!(created.result.profile.revision, 1);

    // Full-document put: re-send the created profile with a new description.
    let mut updated_input = AgentProfileInput {
        profile_id: profile_id.clone(),
        display_name: created.result.profile.display_name.clone(),
        description: created.result.profile.description.clone(),
        document: created.result.profile.document.clone(),
    };
    updated_input.description = Some("Updated live profile".to_owned());
    let updated = api
        .put_profile(ProfilePutParams {
            profile: updated_input,
            expected_revision: Some(1),
        })
        .await?;
    assert_eq!(updated.result.profile.revision, 2);
    assert_eq!(
        updated.result.profile.description.as_deref(),
        Some("Updated live profile")
    );

    let read = api
        .read_profile(ProfileReadParams {
            profile_id: profile_id.clone(),
        })
        .await?;
    assert_eq!(read.result.profile.revision, 2);
    let listed = api.list_profiles(ProfileListParams {}).await?;
    assert!(
        listed
            .result
            .profiles
            .iter()
            .any(|profile| profile.profile_id == profile_id)
    );

    let started = api
        .start_session(SessionStartParams {
            session_id: Some(session_id.as_str().to_owned()),
            display_name: None,
            config: Some(SessionConfig {
                model: Some(model_to_api(&model)),
                ..SessionConfig::default()
            }),
            profile: Some(ProfileSource::Named {
                profile_id: profile_id.clone(),
            }),
        })
        .await?;
    let session = &started.result.session;
    let config = session.config.as_ref().expect("session config");
    let features = config.features.as_ref().expect("session features");
    assert!(features.timers.is_some());
    assert!(features.web.is_none());
    assert_eq!(
        session
            .active_context
            .entries
            .iter()
            .filter(|entry| matches!(&entry.kind, ContextEntryKindView::Instructions))
            .count(),
        1,
        "profile instructions should replace the product fallback"
    );
    assert!(
        session.active_context.entries.iter().any(|entry| matches!(
            &entry.kind,
            ContextEntryKindView::Instructions
        ) && entry.preview.as_deref()
            == Some("Profile instructions")),
        "profile instructions should be projected"
    );

    let mcp_tools: Vec<_> = session
        .active_tools
        .tools
        .iter()
        .filter(|tool| matches!(tool.kind, api::ToolKindView::RemoteMcp { .. }))
        .collect();
    assert_eq!(mcp_tools.len(), 1);
    assert_eq!(mcp_tools[0].tool_id, format!("mcp_{server_id}"));
    let api::ToolKindView::RemoteMcp { server_label, .. } = &mcp_tools[0].kind else {
        panic!("expected remote MCP tool kind");
    };
    assert_eq!(server_label, "profile_crm");

    let applied = api
        .apply_profile(ProfileApplyParams {
            session_id: session_id.as_str().to_owned(),
            profile: ProfileSource::Named {
                profile_id: profile_id.clone(),
            },
            expected_config_revision: Some(session.config_revision),
            expected_tools_revision: Some(session.active_tools.revision),
        })
        .await?;
    assert!(!applied.result.applied.config_changed);
    assert!(!applied.result.applied.instructions_changed);
    assert!(!applied.result.applied.active_environment_changed);

    let cleared = api
        .apply_profile(ProfileApplyParams {
            session_id: session_id.as_str().to_owned(),
            profile: ProfileSource::Inline {
                profile: Box::new(InlineAgentProfile {
                    display_name: Some("No profile instructions".to_owned()),
                    description: None,
                    document: ProfileDocument::default(),
                }),
            },
            expected_config_revision: Some(applied.result.session.config_revision),
            expected_tools_revision: Some(applied.result.session.active_tools.revision),
        })
        .await?;
    assert!(cleared.result.applied.instructions_changed);
    let cleared_instructions = cleared
        .result
        .session
        .active_context
        .entries
        .iter()
        .filter(|entry| matches!(&entry.kind, ContextEntryKindView::Instructions))
        .collect::<Vec<_>>();
    assert_eq!(cleared_instructions.len(), 1);
    assert_ne!(
        cleared_instructions[0].preview.as_deref(),
        Some("Profile instructions")
    );

    let restored = api
        .apply_profile(ProfileApplyParams {
            session_id: session_id.as_str().to_owned(),
            profile: ProfileSource::Named {
                profile_id: profile_id.clone(),
            },
            expected_config_revision: Some(cleared.result.session.config_revision),
            expected_tools_revision: Some(cleared.result.session.active_tools.revision),
        })
        .await?;
    assert!(restored.result.applied.instructions_changed);
    assert_eq!(
        restored
            .result
            .session
            .active_context
            .entries
            .iter()
            .filter(|entry| matches!(&entry.kind, ContextEntryKindView::Instructions))
            .count(),
        1
    );

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "run after profile start".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    let output = final_assistant_text(&run).expect("assistant output");
    assert!(output.contains("Fake agent completed run"));

    api.delete_profile(ProfileDeleteParams { profile_id })
        .await?;
    api.delete_mcp_server(McpServerDeleteParams { server_id })
        .await?;

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent profile live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_openai_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let instructions = "You are Agent in a live integration test. Do not call tools for this test. Reply with the exact phrase requested by the user.";
    let model = openai_live_model();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            ..SessionConfig::default()
        }),
        profile: Some(ProfileSource::Inline {
            profile: Box::new(InlineAgentProfile {
                display_name: Some("OpenAI live test".to_owned()),
                description: None,
                document: ProfileDocument {
                    instructions: Some(ProfileInstructions::Text {
                        text: instructions.to_owned(),
                    }),
                    ..ProfileDocument::default()
                },
            }),
        }),
    })
    .await?;

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "Reply with exactly: real llm agent ok".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    let output = final_assistant_text(&run).expect("OpenAI assistant output");
    let normalized = output.to_lowercase();
    assert!(
        normalized.contains("real llm agent ok"),
        "expected real LLM marker in output: {output}"
    );
    assert!(
        !output.contains("Fake agent completed run"),
        "expected OpenAI-backed output, got fake output: {output}"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent openai live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

async fn run_openai_completions_live_client(
    client: Client,
    task_queue: String,
    session_id: SessionId,
) -> anyhow::Result<()> {
    let store = pg_store_from_env().await?;
    let model = openai_completions_live_model();
    let api = GatewayAgentApi::builder(client.clone(), store)
        .with_task_queue(task_queue)
        .with_default_model(model.clone())
        .build();

    api.start_session(SessionStartParams {
        session_id: Some(session_id.as_str().to_owned()),
        display_name: None,
        config: Some(SessionConfig {
            model: Some(model_to_api(&model)),
            features: Some(FeaturesConfig {
                timers: Some(TimersFeature {
                    version: api::CURRENT_FEATURE_VERSION,
                }),
                ..FeaturesConfig::default()
            }),
            ..SessionConfig::default()
        }),
        profile: Some(ProfileSource::Inline {
            profile: Box::new(InlineAgentProfile {
                display_name: Some("OpenAI Completions tool live test".to_owned()),
                description: None,
                document: ProfileDocument {
                    instructions: Some(ProfileInstructions::Text {
                        text: "You are in a live tool integration test. You must use the requested tools before replying, then follow the exact-output instruction.".to_owned(),
                    }),
                    ..ProfileDocument::default()
                },
            }),
        }),
    })
    .await?;

    let run = api
        .start_run(RunStartParams {
            notify_on_terminal: None,
            submission_id: None,
            session_id: session_id.as_str().to_owned(),
            source: RunStartSource::Input {
                items: vec![InputItem::Text {
                    text: "Call sleep with delay_ms=1, await the returned promise, then reply exactly: completions temporal tool ok".to_owned(),
                }],
            },
            config: None,
        })
        .await?;
    let run = wait_for_terminal_run(&api, &session_id, &run.result.run.id).await?;
    let output = final_assistant_text(&run).expect("OpenAI Completions assistant output");
    assert!(
        output
            .to_lowercase()
            .contains("completions temporal tool ok"),
        "expected completion marker: {output}"
    );
    assert!(
        run.entries.iter().any(|entry| matches!(
            &entry.kind,
            ContextEntryKindView::ToolCall { name, .. } if name == tools::concurrency::SLEEP_TOOL_NAME
        )),
        "expected sleep tool call in run entries: {:?}",
        run.entries
    );
    assert!(
        run.entries.iter().any(|entry| matches!(
            &entry.kind,
            ContextEntryKindView::ToolResult {
                is_error: false,
                ..
            }
        )),
        "expected successful tool result"
    );

    let handle = live_workflow_handle(&client, &session_id)?;
    let _ = handle
        .terminate(
            WorkflowTerminateOptions::builder()
                .reason("agent openai completions live test cleanup")
                .build(),
        )
        .await;
    Ok(())
}

/// P90 isolation: two universes served by ONE worker on ONE shared task
/// queue. The same client-chosen session id exists independently in both
/// universes (distinct composed workflow ids), reads and registry listings
/// never cross universes, and closing one universe's session leaves the
/// other's untouched.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra, Postgres, and Temporal"]
async fn temporal_live_two_universes_share_one_worker_with_isolation() -> anyhow::Result<()> {
    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let task_queue = format!("lightspeed-agent-live-{}", uuid::Uuid::new_v4().simple());
    let temporal_target =
        env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| DEFAULT_TEMPORAL_TARGET.to_owned());
    let namespace =
        env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| DEFAULT_TEMPORAL_NAMESPACE.to_owned());
    let runtime = core_runtime()?;
    let client = connect_temporal(&temporal_target, &namespace).await?;
    let stores = DeploymentStores::from_env().await?;
    let universes = Arc::new(UniverseRuntime::new(
        client.clone(),
        task_queue.clone(),
        None,
        stores,
    )?);

    let activities = WorkerActivities::with_runtime(universes.clone());
    let mut worker =
        worker_with_activities(&runtime, client.clone(), task_queue.clone(), activities)?;
    let shutdown_worker = worker.shutdown_handle();
    let worker_future = worker.run();
    tokio::pin!(worker_future);

    let universe_a = uuid::Uuid::new_v4();
    let universe_b = uuid::Uuid::new_v4();
    let session_id = SessionId::new(format!("session_shared_{}", uuid::Uuid::new_v4().simple()));

    let client_future = async {
        let state_a = universes.state_for(universe_a, true).await?;
        let state_b = universes.state_for(universe_b, true).await?;
        let api_a = state_a.api.clone();
        let api_b = state_b.api.clone();

        // The same client-chosen session id starts independently in both
        // universes on the same queue, served by the same worker.
        for api in [api_a.as_ref(), api_b.as_ref()] {
            let started = api
                .start_session(SessionStartParams {
                    session_id: Some(session_id.as_str().to_owned()),
                    display_name: None,
                    config: None,
                    profile: None,
                })
                .await?;
            assert_eq!(started.result.session.status, SessionStatus::Idle);
        }

        // Distinct workflows: both composed ids are queryable and healthy.
        for universe_id in [universe_a, universe_b] {
            let workflow_id = temporal_workflow::compose_workflow_id(universe_id, &session_id);
            let handle = client.get_workflow_handle::<AgentSessionWorkflow>(workflow_id);
            let status = handle
                .query(
                    AgentSessionWorkflow::status,
                    (),
                    WorkflowQueryOptions::default(),
                )
                .await?;
            assert_eq!(status.last_error, None);
            assert_eq!(status.session_id, session_id.as_str());
        }

        // Registry isolation: a profile created in A is invisible in B, and a
        // read through B reports not-found (no existence leak).
        let profile_id = ProfileId::new(format!("p90.isolation.{}", uuid::Uuid::new_v4().simple()));
        api_a
            .create_profile(ProfileCreateParams {
                profile: AgentProfileInput {
                    profile_id: profile_id.clone(),
                    display_name: Some("P90 isolation".to_owned()),
                    description: None,
                    document: ProfileDocument::default(),
                },
            })
            .await?;
        let listed_b = api_b.list_profiles(ProfileListParams {}).await?;
        assert!(
            listed_b
                .result
                .profiles
                .iter()
                .all(|profile| profile.profile_id != profile_id),
            "universe B must not list universe A's profile"
        );
        let read_b = api_b
            .read_profile(ProfileReadParams {
                profile_id: profile_id.clone(),
            })
            .await;
        match read_b {
            Err(error) => assert_eq!(error.kind, AgentApiErrorKind::NotFound),
            Ok(_) => anyhow::bail!("universe B must not read universe A's profile"),
        }

        // Closing A's session leaves B's session open.
        api_a
            .close_session(api::SessionCloseParams {
                force: false,
                session_id: session_id.as_str().to_owned(),
            })
            .await?;
        let closed_a = api_a
            .read_session(SessionReadParams {
                session_id: session_id.as_str().to_owned(),
            })
            .await?;
        assert_eq!(closed_a.result.session.status, SessionStatus::Closed);
        let open_b = api_b
            .read_session(SessionReadParams {
                session_id: session_id.as_str().to_owned(),
            })
            .await?;
        assert_eq!(open_b.result.session.status, SessionStatus::Idle);

        // Cleanup: terminate both workflows.
        for universe_id in [universe_a, universe_b] {
            let workflow_id = temporal_workflow::compose_workflow_id(universe_id, &session_id);
            let handle = client.get_workflow_handle::<AgentSessionWorkflow>(workflow_id);
            let _ = handle
                .terminate(
                    WorkflowTerminateOptions::builder()
                        .reason("p90 isolation live test cleanup")
                        .build(),
                )
                .await;
        }
        anyhow::Ok(())
    };
    tokio::pin!(client_future);

    let client_result = tokio::select! {
        worker_result = worker_future.as_mut() => {
            return match worker_result {
                Ok(()) => Err(anyhow::anyhow!("Temporal worker stopped before the live test completed")),
                Err(error) => Err(error.context("Temporal worker failed")),
            };
        }
        client_result = client_future.as_mut() => client_result,
    };

    shutdown_worker();
    tokio::time::timeout(Duration::from_secs(10), worker_future.as_mut())
        .await
        .map_err(|_| anyhow::anyhow!("Temporal worker did not shut down within 10 seconds"))??;
    client_result
}

/// P90 Phase 2: `api-key` auth mode end to end over HTTP. Keys resolve to
/// their universe, foreign-universe reads miss, requests without/with bad
/// credentials fail closed, tenant headers are rejected in api-key mode, and
/// revocation takes effect immediately.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires ./dev.sh infra, Postgres, and Temporal"]
async fn temporal_live_api_key_mode_scopes_requests() -> anyhow::Result<()> {
    use auth::ApiKeyStore as _;

    let _lock = LIVE_TEST_LOCK.lock().await;
    let _ = dotenvy::dotenv();
    require_storage_live_env()?;

    let task_queue = format!("lightspeed-agent-live-{}", uuid::Uuid::new_v4().simple());
    let temporal_target =
        env::var("TEMPORAL_ADDRESS").unwrap_or_else(|_| DEFAULT_TEMPORAL_TARGET.to_owned());
    let namespace =
        env::var("TEMPORAL_NAMESPACE").unwrap_or_else(|_| DEFAULT_TEMPORAL_NAMESPACE.to_owned());
    let runtime = core_runtime()?;
    let client = connect_temporal(&temporal_target, &namespace).await?;
    let stores = DeploymentStores::from_env().await?;
    let universes = Arc::new(UniverseRuntime::new(
        client.clone(),
        task_queue.clone(),
        None,
        stores.clone(),
    )?);

    // Two universes, one key each. Keys are minted below the API on purpose:
    // provisioning is out-of-band (server api-key create).
    let universe_a = uuid::Uuid::new_v4();
    let universe_b = uuid::Uuid::new_v4();
    universes.state_for(universe_a, true).await?;
    universes.state_for(universe_b, true).await?;
    let api_keys = store_pg::PgApiKeyStore::new(stores.pool().clone());
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    let minted_a = auth::mint_api_key(
        universe_a,
        auth::PrincipalRef {
            kind: auth::PrincipalKind::ServiceAccount,
            id: Some("live-test".to_owned()),
        },
        Some("p90 live A".to_owned()),
        now_ms,
    );
    let minted_b = auth::mint_api_key(
        universe_b,
        auth::PrincipalRef::universe_default(),
        Some("p90 live B".to_owned()),
        now_ms,
    );
    for minted in [&minted_a, &minted_b] {
        api_keys
            .create_api_key(auth::CreateApiKey {
                key_hash: minted.key_hash.clone(),
                record: minted.record.clone(),
            })
            .await?;
    }

    let activities = WorkerActivities::with_runtime(universes.clone());
    let mut worker =
        worker_with_activities(&runtime, client.clone(), task_queue.clone(), activities)?;
    let shutdown_worker = worker.shutdown_handle();
    let worker_future = worker.run();
    tokio::pin!(worker_future);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let gateway_url = format!("http://{}/rpc", listener.local_addr()?);
    let gateway_state = Arc::new(temporal_server::gateway::GatewayState::multi(
        temporal_server::GatewayAuthMode::ApiKey,
        universes.clone(),
        format!("http://{}", listener.local_addr()?),
    ));
    let gateway = tokio::spawn({
        let gateway_state = gateway_state.clone();
        async move {
            let app = temporal_server::gateway::gateway_router(
                gateway_state,
                temporal_server::gateway::DEFAULT_MAX_REQUEST_BODY_BYTES,
            );
            axum::serve(listener, app).await
        }
    });

    let http = reqwest::Client::new();
    let session_id = format!("session_key_{}", uuid::Uuid::new_v4().simple());
    let rpc = |method: &str, params: serde_json::Value| serde_json::json!({ "id": 1, "method": method, "params": params });
    let call = |bearer: Option<String>, body: serde_json::Value| {
        let http = http.clone();
        let gateway_url = gateway_url.clone();
        async move {
            let mut request = http.post(&gateway_url).json(&body);
            if let Some(bearer) = bearer {
                request = request.header("authorization", format!("Bearer {bearer}"));
            }
            let response: serde_json::Value = request.send().await?.json().await?;
            anyhow::Ok(response)
        }
    };
    let secret_a = minted_a.secret.expose().to_owned();
    let secret_b = minted_b.secret.expose().to_owned();

    let client_future = async {
        // Fail closed: no credential.
        let response = call(
            None,
            rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert!(
            response["error"]["message"]
                .as_str()
                .expect("error message")
                .contains("Authorization")
        );

        // Fail closed: unknown key.
        let response = call(
            Some("lsk_bogus".to_owned()),
            rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert_eq!(
            response["error"]["message"]
                .as_str()
                .expect("error message"),
            "invalid api key"
        );

        // API-key-authenticated tenants can never mint more keys. Operator
        // dispatch is rejected by auth mode before the bearer can select a
        // universe.
        let response = call(
            Some(secret_a.clone()),
            rpc(
                "operator/api-keys/create",
                serde_json::json!({
                    "universeId": universe_a,
                    "displayName": "must not mint",
                    "principal": { "kind": "serviceAccount", "id": "blocked" }
                }),
            ),
        )
        .await?;
        assert_eq!(
            response["error"]["message"]
                .as_str()
                .expect("operator rejection message"),
            "operator methods are not available to api-key callers"
        );

        // Tenant headers are rejected in api-key mode.
        let response: serde_json::Value = http
            .post(&gateway_url)
            .header("authorization", format!("Bearer {secret_a}"))
            .header("x-lightspeed-universe", universe_b.to_string())
            .json(&rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ))
            .send()
            .await?
            .json()
            .await?;
        assert!(
            response["error"]["message"]
                .as_str()
                .expect("error message")
                .contains("x-lightspeed-universe")
        );

        // Key A starts a session in universe A.
        let response = call(
            Some(secret_a.clone()),
            rpc(
                "session/start",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert!(
            response["error"].is_null(),
            "session/start via key A failed: {response}"
        );

        // Key A reads it; key B misses it (not-found, no existence leak).
        let response = call(
            Some(secret_a.clone()),
            rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert!(
            response["error"].is_null(),
            "session/read via key A failed: {response}"
        );
        let response = call(
            Some(secret_b.clone()),
            rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert_eq!(
            response["error"]["code"].as_i64().expect("error code"),
            -32004,
            "key B must not read universe A's session: {response}"
        );

        // The principal of the resolving key is stamped onto grants created
        // through it.
        let response = call(
            Some(secret_a.clone()),
            rpc(
                "auth/grants/import",
                serde_json::json!({ "token": "live-static-token", "displayName": "p90 live" }),
            ),
        )
        .await?;
        assert!(
            response["error"].is_null(),
            "auth/grants/import via key A failed: {response}"
        );
        let state_a = universes.state_for(universe_a, false).await?;
        let grants = auth::AuthGrantStore::list_grants(
            state_a.store.as_ref(),
            auth::ListAuthGrants { status: None },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        let grant = grants.iter().find(|grant| {
            grant.principal.id.as_deref() == Some("live-test")
                && grant.principal.kind == auth::PrincipalKind::ServiceAccount
        });
        assert!(
            grant.is_some(),
            "expected a grant stamped with key A's principal, got: {:?}",
            grants
                .iter()
                .map(|grant| grant.principal.clone())
                .collect::<Vec<_>>()
        );

        // Revocation takes effect immediately.
        assert!(
            api_keys
                .revoke_api_key(&minted_a.record.key_prefix, now_ms + 1)
                .await?
        );
        let response = call(
            Some(secret_a.clone()),
            rpc(
                "session/read",
                serde_json::json!({ "sessionId": session_id }),
            ),
        )
        .await?;
        assert_eq!(
            response["error"]["message"]
                .as_str()
                .expect("error message"),
            "invalid api key"
        );

        // Cleanup.
        let workflow_id = temporal_workflow::compose_workflow_id(
            universe_a,
            &SessionId::new(session_id.as_str()),
        );
        let handle = client.get_workflow_handle::<AgentSessionWorkflow>(workflow_id);
        let _ = handle
            .terminate(
                WorkflowTerminateOptions::builder()
                    .reason("p90 api-key live test cleanup")
                    .build(),
            )
            .await;
        anyhow::Ok(())
    };
    tokio::pin!(client_future);

    let client_result = tokio::select! {
        worker_result = worker_future.as_mut() => {
            return match worker_result {
                Ok(()) => Err(anyhow::anyhow!("Temporal worker stopped before the live test completed")),
                Err(error) => Err(error.context("Temporal worker failed")),
            };
        }
        client_result = client_future.as_mut() => client_result,
    };

    shutdown_worker();
    gateway.abort();
    tokio::time::timeout(Duration::from_secs(10), worker_future.as_mut())
        .await
        .map_err(|_| anyhow::anyhow!("Temporal worker did not shut down within 10 seconds"))??;
    client_result
}
