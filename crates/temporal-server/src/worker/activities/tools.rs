use engine::{
    PromiseControlArgumentCallFacts, PromiseControlArgumentFacts, PromiseControlArgumentRequest,
    PromiseControlKind, ToolBatchOutcome, ToolCallStatus, ToolInvocationResult, storage::BlobStore,
};
use temporalio_sdk::activities::ActivityError;
use tools::concurrency::{CancelArgs, DetachArgs};

use crate::worker::{
    AwaitEnvironmentReadyActivityRequest, AwaitEnvironmentReadyActivityResult, ToolCallExecution,
    ToolInvokeBatchActivityRequest, ToolInvokeCallActivityResult,
};

use super::{
    common::{activity_error, failed_tool_batch_result},
    state::ToolActivityDeps,
};

pub(super) async fn prepare_promise_controls(
    blobs: &dyn BlobStore,
    request: PromiseControlArgumentRequest,
) -> Result<PromiseControlArgumentFacts, ActivityError> {
    if request.version != PromiseControlArgumentRequest::VERSION {
        return Err(activity_error(anyhow::anyhow!(
            "unsupported promise-control argument request version {}",
            request.version
        )));
    }
    let mut calls = Vec::with_capacity(request.calls.len());
    for call in request.calls {
        let parsed = match blobs.read_bytes(&call.arguments_ref).await {
            Ok(bytes) => match call.kind {
                PromiseControlKind::Cancel => serde_json::from_slice::<CancelArgs>(&bytes)
                    .ok()
                    .and_then(|args| args.validated_promise_ids().ok()),
                PromiseControlKind::Detach => serde_json::from_slice::<DetachArgs>(&bytes)
                    .ok()
                    .and_then(|args| args.validated_promise_ids().ok()),
            },
            Err(_) => None,
        };
        calls.push(match parsed {
            Some(promise_ids) => PromiseControlArgumentCallFacts::Parsed {
                call_id: call.call_id,
                promise_ids,
            },
            None => PromiseControlArgumentCallFacts::Invalid {
                call_id: call.call_id,
            },
        });
    }
    Ok(PromiseControlArgumentFacts {
        version: PromiseControlArgumentFacts::VERSION,
        calls,
    })
}

pub(super) async fn invoke_batch(
    deps: &ToolActivityDeps,
    request: ToolInvokeBatchActivityRequest,
) -> Result<ToolBatchOutcome, ActivityError> {
    let request = request.request;
    match deps.tools.invoke_batch(request.clone()).await {
        Ok(result) => Ok(result),
        Err(error) => failed_tool_batch_result(deps.blobs.as_ref(), &request, error.to_string())
            .await
            .map(ToolBatchOutcome::completed)
            .map_err(activity_error),
    }
}

/// Execute one call of an admitted tool batch under its class operation
/// deadline. Tool-level failures and an elapsed operation deadline return an
/// ordinary terminal failed result; only blob-store failures fail the
/// activity. A call whose active environment is still provisioning does not
/// run and reports `EnvironmentNotReady` so the workflow can wait outside
/// this deadline (P125).
pub(super) async fn invoke_call(
    deps: &ToolActivityDeps,
    request: crate::worker::ToolInvokeCallActivityRequest,
) -> Result<ToolInvokeCallActivityResult, ActivityError> {
    let request = request.request;
    let deadline = temporal_workflow::tool_call_operation_timeout(request.execution.class);
    let execution = async {
        if request.remote_mcp.is_some() {
            let native =
                deps.native_mcp
                    .as_ref()
                    .ok_or_else(|| engine::CoreAgentIoError::Failed {
                        message: "native MCP execution is unavailable on this worker".to_owned(),
                    })?;
            let argument_bytes = deps
                .blobs
                .read_bytes(&request.call.arguments_ref)
                .await
                .map_err(|error| engine::CoreAgentIoError::Failed {
                    message: format!("read native MCP arguments: {error}"),
                })?;
            let arguments = serde_json::from_slice(&argument_bytes).map_err(|error| {
                engine::CoreAgentIoError::Failed {
                    message: format!("native MCP arguments are invalid JSON: {error}"),
                }
            })?;
            return match native.execute(&request, arguments).await {
                Ok(crate::worker::mcp::NativeMcpExecutionOutcome::NeedsApproval { subject }) => {
                    Ok(ToolCallExecution::NeedsApproval { subject })
                }
                Ok(crate::worker::mcp::NativeMcpExecutionOutcome::Completed {
                    mut output,
                    visible,
                    is_error,
                    assets,
                }) => {
                    let mut asset_refs = Vec::with_capacity(assets.len());
                    for asset in assets {
                        let blob_ref =
                            deps.blobs.put_bytes(asset.bytes).await.map_err(|error| {
                                engine::CoreAgentIoError::Failed {
                                    message: format!(
                                        "store native MCP {} asset ({}): {error}",
                                        asset.kind,
                                        asset.media_type.as_deref().unwrap_or("unknown media type")
                                    ),
                                }
                            })?;
                        asset_refs.push(blob_ref);
                    }
                    crate::worker::mcp::attach_mcp_asset_refs(&mut output, &asset_refs);
                    let output_ref = deps
                        .blobs
                        .put_bytes(serde_json::to_vec(&output).map_err(|error| {
                            engine::CoreAgentIoError::Failed {
                                message: format!("encode native MCP result: {error}"),
                            }
                        })?)
                        .await
                        .map_err(|error| engine::CoreAgentIoError::Failed {
                            message: format!("store native MCP result: {error}"),
                        })?;
                    let visible_ref =
                        deps.blobs
                            .put_bytes(visible.into_bytes())
                            .await
                            .map_err(|error| engine::CoreAgentIoError::Failed {
                                message: format!("store native MCP visible result: {error}"),
                            })?;
                    let status = if is_error {
                        ToolCallStatus::Failed
                    } else {
                        ToolCallStatus::Succeeded
                    };
                    Ok(ToolCallExecution::Completed(ToolInvocationResult {
                        call_id: request.call.call_id.clone(),
                        status,
                        output_ref: Some(output_ref),
                        model_visible_context_entries: vec![
                            ToolInvocationResult::tool_result_context_entry(
                                &request.call.call_id,
                                status,
                                visible_ref.clone(),
                            ),
                        ],
                        error_ref: is_error.then_some(visible_ref),
                        effects: Vec::new(),
                    }))
                }
                Err(message) => Err(engine::CoreAgentIoError::Failed { message }),
            };
        }
        match deps.hosted.as_ref() {
            Some(hosted) => hosted.invoke_call_execution(request.clone()).await,
            None => deps
                .tools
                .invoke_call(request.clone())
                .await
                .map(ToolCallExecution::Completed),
        }
    };
    match tokio::time::timeout(deadline, execution).await {
        Ok(Ok(ToolCallExecution::Completed(result))) => {
            Ok(ToolInvokeCallActivityResult::Completed { result })
        }
        Ok(Ok(ToolCallExecution::EnvironmentNotReady { environment_id, .. })) => {
            Ok(ToolInvokeCallActivityResult::EnvironmentNotReady { environment_id })
        }
        Ok(Ok(ToolCallExecution::NeedsApproval { subject })) => {
            Ok(ToolInvokeCallActivityResult::NeedsApproval { subject })
        }
        Ok(Err(error)) => {
            super::common::failed_tool_call_result(deps.blobs.as_ref(), &request, error.to_string())
                .await
                .map(|result| ToolInvokeCallActivityResult::Completed { result })
                .map_err(activity_error)
        }
        Err(_elapsed) => super::common::failed_tool_call_result(
            deps.blobs.as_ref(),
            &request,
            format!(
                "tool call exceeded its {}s operation deadline",
                deadline.as_secs()
            ),
        )
        .await
        .map(|result| ToolInvokeCallActivityResult::Completed { result })
        .map_err(activity_error),
    }
}

/// Wait, heartbeating on every poll, until the session's active environment
/// is reachable, terminally unusable, or the bounded readiness window
/// elapses. Runs as its own activity so tool classes keep their deadlines.
pub(super) async fn await_environment_ready(
    deps: &ToolActivityDeps,
    ctx: &temporalio_sdk::activities::ActivityContext,
    request: AwaitEnvironmentReadyActivityRequest,
) -> Result<AwaitEnvironmentReadyActivityResult, ActivityError> {
    let Some(hosted) = deps.hosted.as_ref() else {
        return Ok(AwaitEnvironmentReadyActivityResult::Failed {
            message: "environment readiness is unavailable on this runtime".to_owned(),
        });
    };
    let deadline = tokio::time::Instant::now() + temporal_workflow::ENVIRONMENT_READY_WAIT;
    Ok(hosted
        .await_environment_ready(&request, deadline, || ctx.record_heartbeat(Vec::new()))
        .await)
}

#[cfg(test)]
mod tests {
    use engine::{BlobRef, PromiseControlArgumentCall, ToolCallId, storage::InMemoryBlobStore};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn preparation_reads_only_cas_and_returns_bounded_validated_ids() {
        let blobs = InMemoryBlobStore::new();
        let valid_ref = blobs
            .put_bytes(br#"{"promises":["promise_1","promise_1","promise_2"]}"#.to_vec())
            .await
            .expect("valid args");
        let invalid_ref = blobs
            .put_bytes(br#"{"promises":[]}"#.to_vec())
            .await
            .expect("invalid args");
        let facts = prepare_promise_controls(
            &blobs,
            PromiseControlArgumentRequest {
                version: PromiseControlArgumentRequest::VERSION,
                calls: vec![
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("cancel"),
                        kind: PromiseControlKind::Cancel,
                        arguments_ref: valid_ref,
                    },
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("detach-invalid"),
                        kind: PromiseControlKind::Detach,
                        arguments_ref: invalid_ref,
                    },
                    PromiseControlArgumentCall {
                        call_id: ToolCallId::new("missing"),
                        kind: PromiseControlKind::Cancel,
                        arguments_ref: BlobRef::from_bytes(b"missing"),
                    },
                ],
            },
        )
        .await
        .expect("prepare facts");

        assert_eq!(
            facts.calls,
            vec![
                PromiseControlArgumentCallFacts::Parsed {
                    call_id: ToolCallId::new("cancel"),
                    promise_ids: vec![
                        engine::PromiseId::new("promise_1"),
                        engine::PromiseId::new("promise_2")
                    ],
                },
                PromiseControlArgumentCallFacts::Invalid {
                    call_id: ToolCallId::new("detach-invalid"),
                },
                PromiseControlArgumentCallFacts::Invalid {
                    call_id: ToolCallId::new("missing"),
                },
            ]
        );
    }
}
