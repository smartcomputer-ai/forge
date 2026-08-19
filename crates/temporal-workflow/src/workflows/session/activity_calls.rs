use temporalio_common::error::IncomingError;
use temporalio_sdk::ActivityExecutionError;

use super::*;

/// Longest exhaustion-failure message recorded for the transcript; put_blob
/// inputs stay bounded even for pathological provider error chains.
const MAX_LLM_BOUNDARY_ERROR_BYTES: usize = 16 * 1024;

/// LLM provider activities (P116): terminal provider errors complete the
/// activity with a failed result; transient ones surface as the typed
/// retryable `llm_provider_transient` failure so Temporal owns durable
/// backoff. When the retry budget is exhausted, the recognized failure is
/// converted here into the same terminal result shape the drive consumes —
/// the run fails, the session workflow survives. Anything unrecognized
/// (including cancellation) propagates unchanged so operational bugs stay
/// visible.
/// Run the generation activity while client admissions keep landing
/// A cancel that makes the turn obsolete preempts the call; the
/// engine has already cancelled the turn by then.
pub(super) async fn call_llm_generate(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    request: LlmGenerationRequest,
) -> anyhow::Result<control::Raced<engine::LlmGenerationResult>> {
    let run_id = request.run_id;
    let turn_id = request.turn_id;
    let activity_ctx = ctx.clone();
    let activity = activity_ctx.start_activity(
        WorkflowActivities::llm_generate,
        LlmGenerateActivityRequest { request },
        crate::llm_activity_options(),
    );
    let raced = control::race_activity_with_admissions(ctx, drive, activity, |state| {
        control::generation_still_wanted(state, run_id, turn_id)
    })
    .await?;
    let outcome = match raced {
        control::Raced::Preempted => return Ok(control::Raced::Preempted),
        control::Raced::Completed(outcome) => outcome,
    };
    match outcome {
        Ok(result) => Ok(control::Raced::Completed(result)),
        Err(error) => match llm_transient_exhaustion(&error) {
            Some(details) => {
                let failure_ref =
                    put_llm_boundary_error_blob(ctx, "LLM generation", &details).await;
                Ok(control::Raced::Completed(engine::LlmGenerationResult {
                    run_id,
                    turn_id,
                    status: engine::LlmGenerationStatus::Failed,
                    failure_ref: Some(failure_ref),
                    context_entries: Vec::new(),
                    facts: engine::LlmGenerationFacts {
                        provider_response_id: None,
                        finish: engine::LlmFinish::Failed,
                        usage: None,
                        tool_calls: Vec::new(),
                        context_token_estimate: None,
                    },
                }))
            }
            None => Err(anyhow::anyhow!("{error}")),
        },
    }
}

pub(super) async fn call_context_compact(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    request: engine::ContextCompactionRequest,
) -> anyhow::Result<engine::ContextCompactionResult> {
    let session_id = request.session_id.clone();
    let context_revision = request.request.context.context_revision;
    match ctx
        .start_activity(
            WorkflowActivities::context_compact,
            crate::ContextCompactActivityRequest { request },
            crate::llm_activity_options(),
        )
        .await
    {
        Ok(result) => Ok(result),
        Err(error) => match llm_transient_exhaustion(&error) {
            Some(details) => {
                let failure_ref =
                    put_llm_boundary_error_blob(ctx, "context compaction", &details).await;
                Ok(engine::ContextCompactionResult {
                    session_id,
                    context_revision,
                    status: engine::ContextCompactionStatus::Failed,
                    failure_ref: Some(failure_ref),
                    context_entries: Vec::new(),
                })
            }
            None => Err(anyhow::anyhow!("{error}")),
        },
    }
}

/// Recognizes an exhausted transient provider failure anywhere in the
/// activity failure's cause chain. When schedule-to-close expires during a
/// backoff, the typed application failure arrives as the cause of a timeout
/// failure rather than at the top level. Cancellation is never converted.
fn llm_transient_exhaustion(
    error: &ActivityExecutionError,
) -> Option<crate::LlmTransientFailureDetails> {
    if matches!(error, ActivityExecutionError::Cancelled(_)) {
        return None;
    }
    let mut cause = error.cause();
    while let Some(incoming) = cause {
        if let IncomingError::Application(failure) = incoming
            && failure.type_name() == Some(crate::LLM_PROVIDER_TRANSIENT_ERROR_TYPE)
        {
            let details = failure
                .details::<crate::LlmTransientFailureDetails>()
                .ok()
                .flatten()
                .unwrap_or_else(|| crate::LlmTransientFailureDetails {
                    version: crate::LLM_TRANSIENT_FAILURE_DETAILS_VERSION,
                    message: failure.to_string(),
                    attempt: 0,
                    retry_after_ms: None,
                });
            return Some(details);
        }
        cause = incoming.cause();
    }
    None
}

/// Materialize the exhaustion failure text with bounded attempts; fall back
/// to the well-known engine blob so the failure path itself can never retry
/// unbounded.
async fn put_llm_boundary_error_blob(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    operation: &str,
    details: &crate::LlmTransientFailureDetails,
) -> BlobRef {
    let mut message = format!(
        "{operation} failed: transient provider retries exhausted after {} attempts\nlast provider error: {}\n",
        details.attempt, details.message
    );
    if message.len() > MAX_LLM_BOUNDARY_ERROR_BYTES {
        let mut end = MAX_LLM_BOUNDARY_ERROR_BYTES;
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
        crate::boundary_error_blob_activity_options(),
    )
    .await
    .unwrap_or_else(|_| engine::llm_runtime_boundary_failure_ref())
}

pub(super) async fn call_tool_prepare_promise_controls(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    request: engine::PromiseControlArgumentRequest,
) -> anyhow::Result<engine::PromiseControlArgumentFacts> {
    ctx.start_activity(
        WorkflowActivities::tool_prepare_promise_controls,
        ToolPreparePromiseControlsActivityRequest { request },
        activity_options(),
    )
    .await
    .map_err(|error| anyhow::anyhow!("{error}"))
}
