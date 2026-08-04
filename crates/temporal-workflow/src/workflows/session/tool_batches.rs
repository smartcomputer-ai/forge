//! Per-call tool batch execution (P114).
//!
//! Ordinary batches schedule one Temporal activity per executable call and
//! resume the drive progressively: each terminal call result appends durably
//! on its own, so a slow or failed call never restarts a completed sibling.
//! Batches that need batch-level orchestration (an `await` call or admitted
//! workflow-tool calls) still execute as one unit behind the same
//! progressive-completion engine contract.

use std::pin::Pin;

use engine::{
    AWAIT_TOOL_NAME, ToolCallStatus, ToolExecutionSpec, ToolInvocationResult, ToolName,
    ToolParallelism,
};
use futures::future::select_all;
use temporalio_sdk::ActivityExecutionError;

use crate::{
    MAX_CONCURRENT_TOOL_CALLS_PER_BATCH, ToolInvokeCallActivityRequest,
    boundary_error_blob_activity_options, tool_call_activity_options,
};

use super::*;

/// Longest boundary-failure message recorded for the model; put_blob inputs
/// stay bounded even for pathological error chains.
const MAX_BOUNDARY_ERROR_BYTES: usize = 16 * 1024;

pub(super) async fn invoke_tool_batch(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    request: ToolInvocationBatchRequest,
) -> anyhow::Result<CoreAgentAction> {
    if batch_requires_unit_execution(&request) {
        return invoke_tool_batch_as_unit(ctx, drive, request).await;
    }
    for group in execution_groups(drive.state(), &request) {
        execute_call_group(ctx, drive, &request, group).await?;
    }
    drive
        .next_action_unbounded(workflow_time_ms(ctx))
        .map_err(Into::into)
}

/// `await` defers the whole batch and admitted workflow-tool calls share
/// batch-scoped emission ordering; both stay on the batch-unit activity.
fn batch_requires_unit_execution(request: &ToolInvocationBatchRequest) -> bool {
    request
        .calls
        .iter()
        .any(|call| call.workflow_tool.is_some() || call.tool_name.as_str() == AWAIT_TOOL_NAME)
}

async fn invoke_tool_batch_as_unit(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    request: ToolInvocationBatchRequest,
) -> anyhow::Result<CoreAgentAction> {
    match call_tool_invoke_batch(ctx, request.clone()).await {
        Ok(outcome) => drive
            .resume_tool_batch_outcome(outcome, workflow_time_ms(ctx))
            .map_err(Into::into),
        Err(error) => {
            let status = boundary_call_status(&error);
            let error_ref = put_boundary_error_blob(ctx, &format!("{error}")).await;
            let results = request
                .calls
                .iter()
                .map(|call| boundary_call_result(call.call_id.clone(), status, error_ref.clone()))
                .collect();
            drive
                .resume_tool_batch(
                    engine::ToolInvocationBatchResult {
                        run_id: request.run_id,
                        turn_id: request.turn_id,
                        batch_id: request.batch_id,
                        results,
                    },
                    workflow_time_ms(ctx),
                )
                .map_err(Into::into)
        }
    }
}

/// Execution order over call indices: consecutive parallel-safe calls run
/// concurrently; an exclusive call runs alone, in original batch order.
fn execution_groups(
    state: &CoreAgentState,
    request: &ToolInvocationBatchRequest,
) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut last_parallel_safe = false;
    for (index, call) in request.calls.iter().enumerate() {
        let parallel_safe =
            call_parallelism(state, &call.tool_name) == ToolParallelism::ParallelSafe;
        match groups.last_mut() {
            Some(group) if parallel_safe && last_parallel_safe => group.push(index),
            _ => groups.push(vec![index]),
        }
        last_parallel_safe = parallel_safe;
    }
    groups
}

/// A boxed in-flight call future. `select_all` polls these with the caller's
/// context directly; combinators with internal waker machinery (e.g.
/// `FuturesUnordered`) trip the SDK's nondeterminism detector ([TMPRL1100])
/// and must not be used inside workflow code.
type InflightCall =
    Pin<Box<dyn Future<Output = (usize, Result<ToolInvocationResult, ActivityExecutionError>)>>>;

async fn execute_call_group(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    request: &ToolInvocationBatchRequest,
    group: Vec<usize>,
) -> anyhow::Result<()> {
    let mut pending = group.into_iter();
    let mut inflight: Vec<InflightCall> = Vec::new();
    loop {
        // Topped-up window: at most MAX_CONCURRENT_TOOL_CALLS_PER_BATCH
        // activities execute at once, so one model turn cannot fan out
        // unbounded concurrent work.
        while inflight.len() < MAX_CONCURRENT_TOOL_CALLS_PER_BATCH {
            let Some(index) = pending.next() else { break };
            inflight.push(start_call_activity(ctx, drive.state(), request, index));
        }
        if inflight.is_empty() {
            return Ok(());
        }
        let ((index, outcome), _, remaining) = select_all(inflight).await;
        inflight = remaining;
        resume_call(ctx, drive, request, index, outcome).await?;
    }
}

fn start_call_activity(
    ctx: &WorkflowContext<AgentSessionWorkflow>,
    state: &CoreAgentState,
    request: &ToolInvocationBatchRequest,
    index: usize,
) -> InflightCall {
    let call_ctx = ctx.clone();
    let execution = call_execution_spec(state, &request.calls[index].tool_name);
    let call_request = request
        .call_request(index, execution)
        .expect("group indices come from this batch request");
    Box::pin(async move {
        let outcome = call_ctx
            .start_activity(
                WorkflowActivities::tool_invoke_call,
                ToolInvokeCallActivityRequest {
                    request: call_request,
                },
                tool_call_activity_options(execution),
            )
            .await;
        (index, outcome)
    })
}

/// Accept one call outcome: an activity failure (deadline, exhausted retries,
/// infrastructure) becomes an ordinary terminal failed call result, a
/// cancelled activity records a terminal cancelled result, and a result whose
/// call id does not match the scheduled call is rejected as a boundary
/// failure. The drive then appends the durable per-call completion.
async fn resume_call(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    request: &ToolInvocationBatchRequest,
    index: usize,
    outcome: Result<ToolInvocationResult, ActivityExecutionError>,
) -> anyhow::Result<()> {
    let scheduled_call_id = &request.calls[index].call_id;
    let result = match outcome {
        Ok(result) if result.call_id == *scheduled_call_id => result,
        Ok(result) => {
            let error_ref = put_boundary_error_blob(
                ctx,
                &format!(
                    "tool runtime returned a result for call {} instead of the scheduled call {}",
                    result.call_id, scheduled_call_id
                ),
            )
            .await;
            boundary_call_result(scheduled_call_id.clone(), ToolCallStatus::Failed, error_ref)
        }
        Err(error) => {
            let status = boundary_call_status(&error);
            let error_ref = put_boundary_error_blob(ctx, &format!("{error}")).await;
            boundary_call_result(scheduled_call_id.clone(), status, error_ref)
        }
    };
    let action = drive.resume_tool_call(request.batch_id, result, workflow_time_ms(ctx))?;
    match action {
        CoreAgentAction::AppendEvents {
            expected_head,
            events,
        } => {
            drive::append_events(ctx, drive, expected_head, events).await?;
            Ok(())
        }
        other => anyhow::bail!("tool call resume emitted unexpected action: {other:?}"),
    }
}

/// Cancellation remains cancellation: a cancelled activity records a terminal
/// cancelled call result instead of an ordinary failure.
fn boundary_call_status(error: &ActivityExecutionError) -> ToolCallStatus {
    match error {
        ActivityExecutionError::Cancelled(_) => ToolCallStatus::Cancelled,
        _ => ToolCallStatus::Failed,
    }
}

fn call_execution_spec(state: &CoreAgentState, tool_name: &ToolName) -> ToolExecutionSpec {
    state
        .tooling
        .tools
        .get(tool_name)
        .map(|tool| tool.execution)
        .unwrap_or_default()
}

fn call_parallelism(state: &CoreAgentState, tool_name: &ToolName) -> ToolParallelism {
    state
        .tooling
        .tools
        .get(tool_name)
        .map(|tool| tool.parallelism)
        .unwrap_or(ToolParallelism::Exclusive)
}

/// Materialize the boundary error text with bounded attempts. This path must
/// never reintroduce unlimited retries: when the bounded put fails, fall back
/// to the engine's well-known boundary-failure blob, which every runtime
/// guarantees exists.
async fn put_boundary_error_blob(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    error: &str,
) -> BlobRef {
    let mut message = format!("tool call failed at the runtime boundary: {error}");
    if message.len() > MAX_BOUNDARY_ERROR_BYTES {
        let mut end = MAX_BOUNDARY_ERROR_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    ctx.start_activity(
        WorkflowActivities::put_blob,
        PutBlobRequest {
            bytes: message.into_bytes(),
        },
        boundary_error_blob_activity_options(),
    )
    .await
    .unwrap_or_else(|_| engine::tool_runtime_boundary_failure_ref())
}

fn boundary_call_result(
    call_id: engine::ToolCallId,
    status: ToolCallStatus,
    error_ref: BlobRef,
) -> ToolInvocationResult {
    ToolInvocationResult {
        call_id: call_id.clone(),
        status,
        output_ref: None,
        model_visible_context_entries: vec![ToolInvocationResult::tool_result_context_entry(
            &call_id,
            status,
            error_ref.clone(),
        )],
        error_ref: Some(error_ref),
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use engine::{BlobRef, ToolCallId, ToolInvocationRequest, ToolSpec};

    use super::*;

    fn call(name: &str, id: &str) -> ToolInvocationRequest {
        ToolInvocationRequest {
            call_id: ToolCallId::new(id),
            tool_name: ToolName::new(name),
            arguments_ref: BlobRef::from_bytes(b"{}"),
            workflow_tool: None,
            promise_control: None,
        }
    }

    fn request_with(calls: Vec<ToolInvocationRequest>) -> ToolInvocationBatchRequest {
        ToolInvocationBatchRequest {
            session_id: SessionId::new("session-a"),
            run_id: engine::RunId::new(1),
            turn_id: engine::TurnId::new(1),
            batch_id: engine::ToolBatchId::new(1),
            workspace_links: Vec::new(),
            active_environment_id: None,
            environment_policy: None,
            fleet_policy: None,
            calls,
        }
    }

    fn state_with_tools(specs: Vec<(&str, ToolParallelism)>) -> CoreAgentState {
        let mut state = CoreAgentState::default();
        for (name, parallelism) in specs {
            state.tooling.tools.insert(
                ToolName::new(name),
                ToolSpec {
                    name: ToolName::new(name),
                    kind: engine::ToolKind::Function(engine::FunctionToolSpec {
                        description_ref: None,
                        input_schema_ref: BlobRef::from_bytes(b"{}"),
                        output_schema_ref: None,
                        strict: None,
                        provider_options_ref: None,
                    }),
                    parallelism,
                    execution: ToolExecutionSpec::default(),
                },
            );
        }
        state
    }

    #[test]
    fn await_and_workflow_tool_batches_execute_as_unit() {
        assert!(batch_requires_unit_execution(&request_with(vec![
            call("read_file", "call_read"),
            call("await", "call_await"),
        ])));
        assert!(!batch_requires_unit_execution(&request_with(vec![call(
            "read_file",
            "call_read"
        )])));
    }

    #[test]
    fn execution_groups_run_parallel_safe_runs_together_and_exclusive_alone() {
        let state = state_with_tools(vec![
            ("read_file", ToolParallelism::ParallelSafe),
            ("grep", ToolParallelism::ParallelSafe),
            ("write_file", ToolParallelism::Exclusive),
        ]);
        let request = request_with(vec![
            call("read_file", "call_1"),
            call("grep", "call_2"),
            call("write_file", "call_3"),
            call("read_file", "call_4"),
        ]);

        assert_eq!(
            execution_groups(&state, &request),
            vec![vec![0, 1], vec![2], vec![3]]
        );
    }

    #[test]
    fn unknown_tools_default_to_exclusive_execution() {
        let state = state_with_tools(Vec::new());
        let request = request_with(vec![call("mystery", "call_1"), call("mystery", "call_2")]);

        assert_eq!(execution_groups(&state, &request), vec![vec![0], vec![1]]);
    }
}
