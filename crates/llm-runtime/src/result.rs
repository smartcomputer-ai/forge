use engine::{
    BlobRef, LlmFinish, LlmGenerationFacts, LlmGenerationResult, LlmGenerationStatus, RunId, TurnId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlmGenerationExecution {
    pub result: LlmGenerationResult,
    pub provider_request_ref: BlobRef,
    pub raw_response_ref: BlobRef,
}

pub fn failed_generation_result(run_id: RunId, turn_id: TurnId) -> LlmGenerationResult {
    LlmGenerationResult {
        run_id,
        turn_id,
        status: LlmGenerationStatus::Failed,
        failure_ref: None,
        context_entries: Vec::new(),
        facts: LlmGenerationFacts {
            duration_ms: None,
            provider_response_id: None,
            finish: LlmFinish::Failed,
            usage: None,
            tool_calls: Vec::new(),
            approval_requests: Vec::new(),
            context_token_estimate: None,
        },
    }
}

/// Failure text for a turn the provider cut off at its output cap, in the
/// worker's provider-error blob layout so clients render it like any other
/// model failure. `cap` is the cap the request carried (`None` when the
/// provider applied its own maximum).
pub fn truncation_failure_text(
    run_id: RunId,
    turn_id: TurnId,
    provider: &str,
    response_id: &str,
    cap: Option<u64>,
    output_tokens: Option<u32>,
    reasoning_tokens: Option<u32>,
) -> String {
    let cap = cap
        .map(|cap| cap.to_string())
        .unwrap_or_else(|| "the model maximum".to_owned());
    let spent = match (output_tokens, reasoning_tokens) {
        (Some(output), Some(reasoning)) => {
            format!(" after {output} output tokens ({reasoning} thinking)")
        }
        (Some(output), None) => format!(" after {output} output tokens"),
        _ => String::new(),
    };
    format!(
        "core agent LLM generation failed\nrun_id={run_id}\nturn_id={turn_id}\n\
         error={provider} response {response_id} was cut off at max output tokens {cap}{spent}; \
         the partial output is kept — raise maxOutputTokens or lower reasoningEffort\n"
    )
}

/// The entries of a truncated turn that are safe and useful to keep: the
/// assistant's partial text. Tool calls have no results to replay against,
/// and thinking or provider-opaque blocks from an unfinished turn are not
/// replay-safe.
pub fn partial_output_entries(
    entries: Vec<engine::ContextEntryInput>,
) -> Vec<engine::ContextEntryInput> {
    entries
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                engine::ContextEntryKind::Message {
                    role: engine::ContextMessageRole::Assistant
                }
            )
        })
        .collect()
}
