use engine::{CoreAgentIoError, LlmGenerationResult};
use temporalio_sdk::activities::ActivityError;

use crate::worker::LlmGenerateActivityRequest;

use super::{
    common::{activity_error, failed_generation_result_from_error, transient_provider_failure},
    state::LlmActivityDeps,
};

pub(super) async fn generate(
    deps: &LlmActivityDeps,
    attempt: u32,
    request: LlmGenerateActivityRequest,
) -> Result<LlmGenerationResult, ActivityError> {
    let request = request.request;
    match deps.llm.generate(request.clone()).await {
        Ok(result) => Ok(result),
        // Transient provider errors become the typed retryable activity
        // failure; Temporal owns the durable backoff (P116).
        Err(CoreAgentIoError::Retryable {
            message,
            retry_after,
        }) => Err(transient_provider_failure(
            "LLM generation",
            attempt,
            message,
            retry_after,
        )),
        // Terminal errors complete the activity with a failed generation
        // result and are never retried.
        Err(error) => failed_generation_result_from_error(deps.blobs.as_ref(), request, error)
            .await
            .map_err(activity_error),
    }
}
