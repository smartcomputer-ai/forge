//! OpenAI Chat Completions adapter.
//!
//! Chat Completions has a message-oriented wire format distinct from the
//! Responses API. This adapter lowers engine context directly into native
//! messages and preserves native tool-call objects for exact replay.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use engine::{
    CompactionPolicy, ContextCompactionRequest, ContextCompactionResult, ContextCompactionStatus,
    ContextCompactionTask, ContextEntry, ContextEntryInput, ContextEntryKind, ContextEntrySource,
    ContextMessageRole, LlmFinish, LlmGenerationFacts, LlmGenerationRequest, LlmGenerationResult,
    LlmGenerationStatus, LlmRequest, LlmUsage, OPENAI_COMPLETIONS_COMPACTION_PROVIDER_KIND,
    ObservedToolCall, ProviderApiKind, TokenEstimate, TokenEstimateQuality, ToolCallId, ToolChoice,
    ToolKind, ToolName, ToolSpec, storage::BlobStore,
};
use llm_clients::{ApiResponse, openai::completions as oai_c};
use serde_json::{Value, json};

use crate::{
    blob_io::{put_json, put_text, read_json, read_text},
    error::{LlmAdapterError, LlmAdapterResult},
    executor::{LlmCompactionAdapter, LlmGenerationAdapter},
    params::{openai_completions_params, validate_openai_reasoning_effort},
    provider_keys::{NoStoredProviderKeys, ProviderKeyResolver, resolve_stored_provider_key},
    result::LlmGenerationExecution,
};

pub const OPENAI_COMPLETIONS_MESSAGE_PROVIDER_KIND: &str = "openai.completions.message";
pub const OPENAI_COMPLETIONS_REFUSAL_PROVIDER_KIND: &str = "openai.completions.refusal";
pub const OPENAI_COMPLETIONS_TOOL_CALL_PROVIDER_KIND: &str = "openai.completions.tool_call";

const MEDIA_TYPE_JSON: &str = "application/json";
const MEDIA_TYPE_TEXT: &str = "text/plain";
const DEFAULT_COMPACTION_MAX_TOKENS: u64 = 2048;
const COMPACTION_INSTRUCTION: &str = "Summarize the conversation above for context compaction. \
Capture the user's goals, decisions made, work completed, important tool results, and open \
questions. The summary will replace the prior conversation history, so include everything needed \
to continue seamlessly. Reply with the summary only.";

#[async_trait]
pub trait OpenAiCompletionsApi: Send + Sync {
    async fn create(
        &self,
        request: oai_c::CreateCompletionRequest,
        auth: Option<llm_clients::RequestAuth<'_>>,
    ) -> Result<ApiResponse<oai_c::Completion>, llm_clients::LlmApiError>;
}

#[async_trait]
impl OpenAiCompletionsApi for oai_c::Client {
    async fn create(
        &self,
        request: oai_c::CreateCompletionRequest,
        auth: Option<llm_clients::RequestAuth<'_>>,
    ) -> Result<ApiResponse<oai_c::Completion>, llm_clients::LlmApiError> {
        oai_c::Client::create_with_auth(self, request, auth).await
    }
}

#[derive(Clone)]
pub struct OpenAiCompletionsLlmAdapter {
    client: Arc<dyn OpenAiCompletionsApi>,
    blobs: Arc<dyn BlobStore>,
    provider_keys: Arc<dyn ProviderKeyResolver>,
}

impl OpenAiCompletionsLlmAdapter {
    pub fn new(client: Arc<dyn OpenAiCompletionsApi>, blobs: Arc<dyn BlobStore>) -> Self {
        Self {
            client,
            blobs,
            provider_keys: Arc::new(NoStoredProviderKeys),
        }
    }

    pub fn with_provider_key_resolver(
        mut self,
        provider_keys: Arc<dyn ProviderKeyResolver>,
    ) -> Self {
        self.provider_keys = provider_keys;
        self
    }

    pub async fn materialize_create_request(
        &self,
        request: &LlmRequest,
    ) -> LlmAdapterResult<oai_c::CreateCompletionRequest> {
        materialize_create_request(self.blobs.as_ref(), request).await
    }

    pub async fn materialize_compact_request(
        &self,
        task: &ContextCompactionTask,
    ) -> LlmAdapterResult<oai_c::CreateCompletionRequest> {
        materialize_compact_request(self.blobs.as_ref(), task).await
    }
}

#[async_trait]
impl LlmGenerationAdapter for OpenAiCompletionsLlmAdapter {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> LlmAdapterResult<LlmGenerationExecution> {
        if request.request.model.api_kind != ProviderApiKind::OpenAiCompletions {
            return Err(LlmAdapterError::RequestKindMismatch {
                message: format!(
                    "expected OpenAiCompletions request, got {:?}",
                    request.request.model.api_kind
                ),
            });
        }
        let provider_request = self.materialize_create_request(&request.request).await?;
        let stored_key =
            resolve_stored_provider_key(self.provider_keys.as_ref(), &request.request.model)
                .await?;
        let provider_request_ref = put_json(self.blobs.as_ref(), &provider_request).await?;
        let response = self
            .client
            .create(
                provider_request,
                stored_key.as_ref().map(|auth| auth.as_request_auth()),
            )
            .await?;
        let raw_response_ref = put_json(self.blobs.as_ref(), &response.raw_json).await?;
        let result = result_from_response(self.blobs.as_ref(), &request, &response).await?;
        Ok(LlmGenerationExecution {
            result,
            provider_request_ref,
            raw_response_ref,
        })
    }
}

#[async_trait]
impl LlmCompactionAdapter for OpenAiCompletionsLlmAdapter {
    async fn compact_context(
        &self,
        request: ContextCompactionRequest,
    ) -> LlmAdapterResult<ContextCompactionResult> {
        if request.request.model.api_kind != ProviderApiKind::OpenAiCompletions {
            return Err(LlmAdapterError::RequestKindMismatch {
                message: format!(
                    "expected OpenAiCompletions compaction task, got {:?}",
                    request.request.model.api_kind
                ),
            });
        }
        let provider_request = self.materialize_compact_request(&request.request).await?;
        let stored_key =
            resolve_stored_provider_key(self.provider_keys.as_ref(), &request.request.model)
                .await?;
        let _provider_request_ref = put_json(self.blobs.as_ref(), &provider_request).await?;
        let response = self
            .client
            .create(
                provider_request,
                stored_key.as_ref().map(|auth| auth.as_request_auth()),
            )
            .await?;
        let _raw_response_ref = put_json(self.blobs.as_ref(), &response.raw_json).await?;
        result_from_compact_response(self.blobs.as_ref(), &request, &response).await
    }
}

pub async fn materialize_create_request(
    blobs: &dyn BlobStore,
    request: &LlmRequest,
) -> LlmAdapterResult<oai_c::CreateCompletionRequest> {
    if request.provider_response_id.is_some() {
        return Err(LlmAdapterError::InvalidProviderRequest {
            message: "Chat Completions has no provider response continuation; provider_response_id must be empty"
                .to_owned(),
        });
    }
    if matches!(
        request.compaction,
        Some(CompactionPolicy::ProviderTriggered { .. })
    ) {
        return Err(LlmAdapterError::InvalidProviderRequest {
            message: "Chat Completions does not support provider-triggered compaction".to_owned(),
        });
    }

    let params = openai_completions_params(request.params.as_ref())?;
    let reasoning_effort = request
        .reasoning_effort
        .as_deref()
        .map(validate_openai_reasoning_effort)
        .transpose()?;
    let parallel_tool_calls = params.parallel_tool_calls.or(request.parallel_tool_use);

    Ok(oai_c::CreateCompletionRequest {
        model: request.model.model.clone(),
        messages: materialize_messages(blobs, &request.context.entries).await?,
        tools: materialize_tools(blobs, &request.tools).await?,
        tool_choice: request.tool_choice.as_ref().map(openai_tool_choice),
        response_format: params.response_format,
        temperature: optional_f64(params.temperature.as_ref(), "temperature")?,
        top_p: optional_f64(params.top_p.as_ref(), "top_p")?,
        max_tokens: None,
        max_completion_tokens: request.output_limit.map(u64::from),
        stop: params.stop,
        parallel_tool_calls,
        store: params.store,
        stream: Some(false),
        stream_options: None,
        metadata: non_empty_map(params.metadata),
        reasoning_effort,
        extra: params.extra,
    })
}

pub async fn materialize_compact_request(
    blobs: &dyn BlobStore,
    task: &ContextCompactionTask,
) -> LlmAdapterResult<oai_c::CreateCompletionRequest> {
    let mut messages = materialize_messages(blobs, &task.context.entries).await?;
    messages.push(oai_c::CompletionMessage::user(compaction_instruction(
        task.target_tokens,
    )));
    Ok(oai_c::CreateCompletionRequest {
        model: task.model.model.clone(),
        messages,
        max_completion_tokens: Some(
            task.target_tokens
                .map(u64::from)
                .unwrap_or(DEFAULT_COMPACTION_MAX_TOKENS),
        ),
        stream: Some(false),
        ..Default::default()
    })
}

fn compaction_instruction(target_tokens: Option<u32>) -> String {
    match target_tokens {
        Some(tokens) => format!("{COMPACTION_INSTRUCTION} Keep the summary under {tokens} tokens."),
        None => COMPACTION_INSTRUCTION.to_owned(),
    }
}

async fn materialize_messages(
    blobs: &dyn BlobStore,
    entries: &[ContextEntry],
) -> LlmAdapterResult<Vec<oai_c::CompletionMessage>> {
    let mut messages = Vec::new();
    let mut last_assistant_source: Option<ContextEntrySource> = None;

    for entry in entries {
        match &entry.kind {
            ContextEntryKind::ToolCall { .. } => {
                require_completions_provider_kind(entry)?;
                let raw = read_json(blobs, &entry.content_ref).await?;
                let tool_call: oai_c::CompletionToolCall =
                    serde_json::from_value(raw).map_err(|error| {
                        LlmAdapterError::InvalidProviderRequest {
                            message: format!(
                                "Chat Completions tool-call entry {} is invalid: {error}",
                                entry.entry_id
                            ),
                        }
                    })?;
                let can_fold = messages
                    .last()
                    .is_some_and(|message: &oai_c::CompletionMessage| message.role == "assistant")
                    && last_assistant_source.as_ref() == Some(&entry.source);
                if can_fold {
                    messages
                        .last_mut()
                        .expect("assistant present")
                        .tool_calls
                        .get_or_insert_with(Vec::new)
                        .push(tool_call);
                } else {
                    messages.push(oai_c::CompletionMessage {
                        role: "assistant".to_owned(),
                        tool_calls: Some(vec![tool_call]),
                        ..Default::default()
                    });
                }
                last_assistant_source = Some(entry.source.clone());
            }
            ContextEntryKind::ReasoningState | ContextEntryKind::ProviderOpaque => {
                require_completions_provider_kind(entry)?;
                if entry.media_type.as_deref() != Some(MEDIA_TYPE_JSON) {
                    return Err(LlmAdapterError::InvalidProviderRequest {
                        message: format!(
                            "Chat Completions native entry {} must contain JSON",
                            entry.entry_id
                        ),
                    });
                }
                let raw = read_json(blobs, &entry.content_ref).await?;
                let message: oai_c::CompletionMessage =
                    serde_json::from_value(raw).map_err(|error| {
                        LlmAdapterError::InvalidProviderRequest {
                            message: format!(
                                "Chat Completions native entry {} is not a message: {error}",
                                entry.entry_id
                            ),
                        }
                    })?;
                last_assistant_source = (message.role == "assistant").then(|| entry.source.clone());
                messages.push(message);
            }
            _ => {
                reject_foreign_provider_kind(entry)?;
                let message = materialize_message(blobs, entry).await?;
                let assistant = message.role == "assistant";
                push_message(&mut messages, message);
                last_assistant_source = assistant.then(|| entry.source.clone());
            }
        }
    }
    Ok(messages)
}

fn reject_foreign_provider_kind(entry: &ContextEntry) -> LlmAdapterResult<()> {
    let Some(kind) = entry.provider_kind.as_deref() else {
        return Ok(());
    };
    if (kind.starts_with("openai.") || kind.starts_with("anthropic."))
        && !kind.starts_with("openai.completions.")
    {
        return Err(LlmAdapterError::RequestKindMismatch {
            message: format!(
                "context entry {} has provider kind {kind:?}, expected openai.completions.*",
                entry.entry_id
            ),
        });
    }
    Ok(())
}

fn require_completions_provider_kind(entry: &ContextEntry) -> LlmAdapterResult<()> {
    if entry
        .provider_kind
        .as_deref()
        .is_some_and(|kind| kind.starts_with("openai.completions."))
    {
        Ok(())
    } else {
        Err(LlmAdapterError::RequestKindMismatch {
            message: format!(
                "context entry {} has provider kind {:?}, expected openai.completions.*",
                entry.entry_id, entry.provider_kind
            ),
        })
    }
}

fn push_message(
    messages: &mut Vec<oai_c::CompletionMessage>,
    mut message: oai_c::CompletionMessage,
) {
    if message.role == "user"
        && let Some(previous) = messages.last_mut()
        && previous.role == "user"
    {
        let mut parts = take_parts(previous.content.take());
        parts.extend(take_parts(message.content.take()));
        previous.content = Some(oai_c::CompletionMessageContent::Parts(parts));
        return;
    }
    messages.push(message);
}

fn take_parts(content: Option<oai_c::CompletionMessageContent>) -> Vec<oai_c::CompletionContent> {
    match content {
        Some(oai_c::CompletionMessageContent::Text(text)) => vec![text_part(text)],
        Some(oai_c::CompletionMessageContent::Parts(parts)) => parts,
        None => Vec::new(),
    }
}

async fn materialize_message(
    blobs: &dyn BlobStore,
    entry: &ContextEntry,
) -> LlmAdapterResult<oai_c::CompletionMessage> {
    match &entry.kind {
        ContextEntryKind::Message { role } => {
            let role = match role {
                ContextMessageRole::User => "user",
                ContextMessageRole::Assistant => "assistant",
            };
            let content = if let Some(mime) =
                crate::blob_io::image_media_type(entry.media_type.as_deref())
            {
                let data = crate::blob_io::read_base64(blobs, &entry.content_ref).await?;
                oai_c::CompletionMessageContent::Parts(vec![part_with_extra(
                    "image_url",
                    "image_url",
                    json!({ "url": format!("data:{mime};base64,{data}") }),
                )])
            } else if let Some(document) = crate::blob_io::document_entry(
                entry.media_type.as_deref(),
                entry.preview.as_deref(),
            ) {
                if document.is_pdf {
                    let data = crate::blob_io::read_base64(blobs, &entry.content_ref).await?;
                    oai_c::CompletionMessageContent::Parts(vec![part_with_extra(
                        "file",
                        "file",
                        json!({
                            "filename": document.name.unwrap_or_else(|| "document.pdf".to_owned()),
                            "file_data": format!("data:{};base64,{data}", document.mime),
                        }),
                    )])
                } else {
                    let text = read_text(blobs, &entry.content_ref).await?;
                    let header = document
                        .name
                        .map(|name| format!("[document: {name}]"))
                        .unwrap_or_else(|| "[document]".to_owned());
                    oai_c::CompletionMessageContent::Text(format!("{header}\n\n{text}"))
                }
            } else {
                oai_c::CompletionMessageContent::Text(read_text(blobs, &entry.content_ref).await?)
            };
            let refusal = (entry.provider_kind.as_deref()
                == Some(OPENAI_COMPLETIONS_REFUSAL_PROVIDER_KIND))
            .then(|| match &content {
                oai_c::CompletionMessageContent::Text(text) => text.clone(),
                oai_c::CompletionMessageContent::Parts(_) => String::new(),
            });
            Ok(oai_c::CompletionMessage {
                role: role.to_owned(),
                content: Some(content),
                refusal,
                ..Default::default()
            })
        }
        ContextEntryKind::Instructions => Ok(text_message(
            "developer",
            read_text(blobs, &entry.content_ref).await?,
        )),
        ContextEntryKind::VfsCatalog => {
            let catalog =
                crate::environment_prompts::read_vfs_catalog(blobs, &entry.content_ref).await?;
            Ok(text_message(
                "developer",
                crate::environment_prompts::vfs_catalog_text(&catalog),
            ))
        }
        ContextEntryKind::SkillCatalog => {
            let catalog =
                crate::skill_prompts::read_skill_catalog(blobs, &entry.content_ref).await?;
            Ok(text_message(
                "developer",
                crate::skill_prompts::skill_catalog_text(&catalog),
            ))
        }
        ContextEntryKind::SkillActivation { skill_id, .. } => Ok(text_message(
            "developer",
            crate::skill_prompts::skill_activation_text(
                skill_id,
                read_text(blobs, &entry.content_ref).await?,
            ),
        )),
        ContextEntryKind::ToolResult { call_id, .. } => Ok(oai_c::CompletionMessage {
            role: "tool".to_owned(),
            content: Some(oai_c::CompletionMessageContent::Text(
                read_text(blobs, &entry.content_ref).await?,
            )),
            tool_call_id: Some(call_id.as_str().to_owned()),
            ..Default::default()
        }),
        ContextEntryKind::ToolCall { .. }
        | ContextEntryKind::ReasoningState
        | ContextEntryKind::ProviderOpaque => unreachable!("handled by materialize_messages"),
    }
}

fn text_message(role: &str, text: String) -> oai_c::CompletionMessage {
    oai_c::CompletionMessage {
        role: role.to_owned(),
        content: Some(oai_c::CompletionMessageContent::Text(text)),
        ..Default::default()
    }
}

fn text_part(text: String) -> oai_c::CompletionContent {
    oai_c::CompletionContent {
        r#type: "text".to_owned(),
        text: Some(text),
        ..Default::default()
    }
}

fn part_with_extra(kind: &str, key: &str, value: Value) -> oai_c::CompletionContent {
    let mut extra = BTreeMap::new();
    extra.insert(key.to_owned(), value);
    oai_c::CompletionContent {
        r#type: kind.to_owned(),
        extra,
        ..Default::default()
    }
}

async fn materialize_tools(
    blobs: &dyn BlobStore,
    tools: &[ToolSpec],
) -> LlmAdapterResult<Option<Vec<oai_c::CompletionTool>>> {
    let mut materialized = Vec::new();
    for tool in tools {
        match &tool.kind {
            ToolKind::Function(function) => {
                let mut definition = oai_c::CompletionFunction {
                    name: tool.name.as_str().to_owned(),
                    description: match &function.description_ref {
                        Some(blob_ref) => Some(read_text(blobs, blob_ref).await?),
                        None => None,
                    },
                    parameters: Some(read_json(blobs, &function.input_schema_ref).await?),
                    strict: function.strict,
                    extra: Default::default(),
                };
                if let Some(options_ref) = &function.provider_options_ref {
                    let options = read_json(blobs, options_ref).await?;
                    let Some(options) = options.as_object() else {
                        return Err(LlmAdapterError::InvalidProviderRequest {
                            message: format!(
                                "provider options for tool {} must be a JSON object",
                                tool.name
                            ),
                        });
                    };
                    definition.extra.extend(options.clone());
                }
                materialized.push(oai_c::CompletionTool {
                    r#type: oai_c::CompletionToolType::Function,
                    function: definition,
                });
            }
            ToolKind::ProviderNative(_) | ToolKind::RemoteMcp(_) => {
                return Err(LlmAdapterError::InvalidProviderRequest {
                    message: format!(
                        "tool {} is not expressible by openai:completions",
                        tool.name
                    ),
                });
            }
        }
    }
    Ok((!materialized.is_empty()).then_some(materialized))
}

fn openai_tool_choice(choice: &ToolChoice) -> oai_c::CompletionToolChoice {
    match choice {
        ToolChoice::Auto => {
            oai_c::CompletionToolChoice::Mode(oai_c::CompletionToolChoiceMode::Auto)
        }
        ToolChoice::None => {
            oai_c::CompletionToolChoice::Mode(oai_c::CompletionToolChoiceMode::None)
        }
        ToolChoice::RequiredAny => {
            oai_c::CompletionToolChoice::Mode(oai_c::CompletionToolChoiceMode::Required)
        }
        ToolChoice::Specific { tool_name } => oai_c::CompletionToolChoice::Function {
            r#type: oai_c::CompletionToolType::Function,
            function: oai_c::CompletionToolChoiceFunction {
                name: tool_name.as_str().to_owned(),
            },
        },
    }
}

pub async fn result_from_response(
    blobs: &dyn BlobStore,
    request: &LlmGenerationRequest,
    response: &ApiResponse<oai_c::Completion>,
) -> LlmAdapterResult<LlmGenerationResult> {
    let choice =
        response
            .parsed
            .choices
            .first()
            .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
                message: format!("Chat completion {} has no choices", response.parsed.id),
            })?;
    let message =
        choice
            .message
            .as_ref()
            .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
                message: format!("Chat completion {} has no message", response.parsed.id),
            })?;

    let mut context_entries = Vec::new();
    let mut tool_calls = Vec::new();
    let text = message.text();
    let refusal = message_refusal(message);
    let visible = if text.is_empty() {
        refusal.as_deref().unwrap_or("")
    } else {
        &text
    };
    if !visible.is_empty() {
        let content_ref = put_text(blobs, visible).await?;
        context_entries.push(ContextEntryInput {
            kind: ContextEntryKind::Message {
                role: ContextMessageRole::Assistant,
            },
            content_ref,
            media_type: Some(MEDIA_TYPE_TEXT.to_owned()),
            preview: Some(visible.to_owned()),
            provider_kind: Some(
                if text.is_empty() && refusal.is_some() {
                    OPENAI_COMPLETIONS_REFUSAL_PROVIDER_KIND
                } else {
                    OPENAI_COMPLETIONS_MESSAGE_PROVIDER_KIND
                }
                .to_owned(),
            ),
            provider_item_id: None,
            token_estimate: None,
        });
    }

    for (index, call) in message.tool_calls.iter().flatten().enumerate() {
        let raw_call = raw_tool_call(&response.raw_json, index, call)?;
        let (entry, observed) = tool_call_context(blobs, call, raw_call).await?;
        context_entries.push(entry);
        tool_calls.push(observed);
    }

    let usage = response.parsed.usage.as_ref().map(llm_usage);
    let context_token_estimate = response
        .parsed
        .usage
        .as_ref()
        .and_then(|usage| usage.prompt_tokens)
        .map(|tokens| TokenEstimate {
            tokens: u64_to_u32(tokens),
            quality: TokenEstimateQuality::ProviderCounted,
        });
    Ok(LlmGenerationResult {
        run_id: request.run_id,
        turn_id: request.turn_id,
        status: LlmGenerationStatus::Succeeded,
        failure_ref: None,
        context_entries,
        facts: LlmGenerationFacts {
            provider_response_id: Some(response.parsed.id.clone()),
            finish: finish_reason(choice.finish_reason.as_deref(), !tool_calls.is_empty()),
            usage,
            tool_calls,
            context_token_estimate,
        },
    })
}

pub async fn result_from_compact_response(
    blobs: &dyn BlobStore,
    request: &ContextCompactionRequest,
    response: &ApiResponse<oai_c::Completion>,
) -> LlmAdapterResult<ContextCompactionResult> {
    let summary = response.parsed.output_text();
    let summary = summary.trim();
    if summary.is_empty() {
        return Err(LlmAdapterError::InvalidProviderRequest {
            message: format!(
                "Chat Completions compaction response {} did not include summary text",
                response.parsed.id
            ),
        });
    }
    let content_ref = put_text(blobs, summary).await?;
    Ok(ContextCompactionResult {
        session_id: request.session_id.clone(),
        context_revision: request.request.context.context_revision,
        status: ContextCompactionStatus::Succeeded,
        failure_ref: None,
        context_entries: vec![ContextEntryInput {
            kind: ContextEntryKind::Message {
                role: ContextMessageRole::User,
            },
            content_ref,
            media_type: Some(MEDIA_TYPE_TEXT.to_owned()),
            preview: Some(summary.to_owned()),
            provider_kind: Some(OPENAI_COMPLETIONS_COMPACTION_PROVIDER_KIND.to_owned()),
            provider_item_id: Some(response.parsed.id.clone()),
            token_estimate: None,
        }],
    })
}

fn message_refusal(message: &oai_c::CompletionMessage) -> Option<String> {
    message.refusal.clone().or_else(|| match &message.content {
        Some(oai_c::CompletionMessageContent::Parts(parts)) => {
            let refusal = parts
                .iter()
                .filter_map(|part| part.refusal.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            (!refusal.is_empty()).then_some(refusal)
        }
        _ => None,
    })
}

fn raw_tool_call(
    raw_response: &Value,
    index: usize,
    call: &oai_c::CompletionToolCall,
) -> LlmAdapterResult<Value> {
    if let Some(raw) = raw_response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
        .and_then(|calls| calls.get(index))
    {
        return Ok(raw.clone());
    }
    serde_json::to_value(call).map_err(|error| LlmAdapterError::InvalidProviderRequest {
        message: format!("failed to encode Chat Completions tool call: {error}"),
    })
}

async fn tool_call_context(
    blobs: &dyn BlobStore,
    call: &oai_c::CompletionToolCall,
    raw_call: Value,
) -> LlmAdapterResult<(ContextEntryInput, ObservedToolCall)> {
    let id = call
        .id
        .as_deref()
        .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
            message: "Chat Completions tool call is missing id".to_owned(),
        })?;
    let call_id = ToolCallId::try_new(id.to_owned()).map_err(|error| {
        LlmAdapterError::InvalidProviderRequest {
            message: format!("invalid Chat Completions tool call id {id:?}: {error}"),
        }
    })?;
    let function =
        call.function
            .as_ref()
            .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
                message: format!("Chat Completions tool call {id} has no function"),
            })?;
    let name = function
        .name
        .as_deref()
        .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
            message: format!("Chat Completions tool call {id} has no function name"),
        })?;
    let tool_name = ToolName::try_new(name.to_owned()).map_err(|error| {
        LlmAdapterError::InvalidProviderRequest {
            message: format!("invalid Chat Completions tool name {name:?}: {error}"),
        }
    })?;
    let raw_arguments = function.arguments.as_deref().unwrap_or("{}");
    let arguments =
        serde_json::from_str(raw_arguments).unwrap_or_else(|_| json!({ "__raw": raw_arguments }));
    let arguments_ref = put_json(blobs, &arguments).await?;
    let native_call_ref = put_json(blobs, &raw_call).await?;
    Ok((
        ContextEntryInput {
            kind: ContextEntryKind::ToolCall {
                call_id: call_id.clone(),
                name: tool_name.clone(),
            },
            content_ref: native_call_ref.clone(),
            media_type: Some(MEDIA_TYPE_JSON.to_owned()),
            preview: Some(format!("{tool_name}({raw_arguments})")),
            provider_kind: Some(OPENAI_COMPLETIONS_TOOL_CALL_PROVIDER_KIND.to_owned()),
            provider_item_id: Some(id.to_owned()),
            token_estimate: None,
        },
        ObservedToolCall {
            call_id,
            tool_name,
            provider_kind: Some(OPENAI_COMPLETIONS_TOOL_CALL_PROVIDER_KIND.to_owned()),
            arguments_ref,
            native_call_ref: Some(native_call_ref),
        },
    ))
}

fn finish_reason(reason: Option<&str>, has_tool_calls: bool) -> LlmFinish {
    match reason {
        Some("tool_calls" | "function_call") => LlmFinish::ToolCalls,
        Some("stop") => LlmFinish::Stop,
        Some("length") => LlmFinish::Length,
        Some("content_filter") => LlmFinish::ContentFilter,
        Some(_) => LlmFinish::Unknown,
        None if has_tool_calls => LlmFinish::ToolCalls,
        None => LlmFinish::Unknown,
    }
}

fn llm_usage(usage: &oai_c::CompletionUsage) -> LlmUsage {
    LlmUsage {
        input_tokens: usage.prompt_tokens.map(u64_to_u32),
        output_tokens: usage.completion_tokens.map(u64_to_u32),
        reasoning_tokens: usage.reasoning_tokens().map(u64_to_u32),
        total_tokens: usage.total_tokens.map(u64_to_u32),
    }
}

fn optional_f64(value: Option<&Value>, name: &'static str) -> LlmAdapterResult<Option<f64>> {
    value
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| LlmAdapterError::InvalidProviderRequest {
                    message: format!("{name} must be a JSON number"),
                })
        })
        .transpose()
}

fn non_empty_map<K, V>(map: BTreeMap<K, V>) -> Option<BTreeMap<K, V>> {
    (!map.is_empty()).then_some(map)
}

fn u64_to_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use engine::{
        BlobRef, ContextEntryId, ContextSnapshot, ModelSelection, ProviderParams, RunId, SessionId,
        TurnId,
        storage::{BlobStore, InMemoryBlobStore},
    };
    use llm_clients::HeaderSnapshot;
    use serde_json::json;

    use super::*;

    struct FakeOpenAiCompletionsApi {
        response: ApiResponse<oai_c::Completion>,
        seen_auth: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl OpenAiCompletionsApi for FakeOpenAiCompletionsApi {
        async fn create(
            &self,
            _request: oai_c::CreateCompletionRequest,
            auth: Option<llm_clients::RequestAuth<'_>>,
        ) -> Result<ApiResponse<oai_c::Completion>, llm_clients::LlmApiError> {
            self.seen_auth
                .lock()
                .expect("lock")
                .push(auth.map(|auth| match auth {
                    llm_clients::RequestAuth::ApiKey(value) => format!("api_key:{value}"),
                    llm_clients::RequestAuth::Bearer(value) => format!("bearer:{value}"),
                }));
            Ok(self.response.clone())
        }
    }

    fn fake_api() -> Arc<FakeOpenAiCompletionsApi> {
        let raw = json!({
            "id": "chatcmpl_fake",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "done" }
            }]
        });
        Arc::new(FakeOpenAiCompletionsApi {
            response: ApiResponse {
                parsed: serde_json::from_value(raw.clone()).expect("response"),
                raw_json: raw,
                status: 200,
                headers: HeaderSnapshot::default(),
            },
            seen_auth: Mutex::new(Vec::new()),
        })
    }

    fn model() -> ModelSelection {
        ModelSelection {
            api_kind: ProviderApiKind::OpenAiCompletions,
            provider_id: "openai".to_owned(),
            model: "gpt-5.1".to_owned(),
        }
    }

    fn request(entries: Vec<ContextEntry>) -> LlmRequest {
        LlmRequest {
            model: model(),
            request_fingerprint: "sha256:test".to_owned(),
            context: ContextSnapshot {
                api_kind: ProviderApiKind::OpenAiCompletions,
                context_revision: 7,
                entries,
                token_estimate: None,
            },
            tools: Vec::new(),
            tool_choice: None,
            output_limit: None,
            reasoning_effort: None,
            parallel_tool_use: None,
            provider_response_id: None,
            compaction: None,
            params: None,
        }
    }

    fn entry(
        id: u64,
        kind: ContextEntryKind,
        source: ContextEntrySource,
        content_ref: BlobRef,
    ) -> ContextEntry {
        ContextEntry {
            entry_id: ContextEntryId::new(id),
            key: None,
            kind,
            source,
            content_ref,
            media_type: Some(MEDIA_TYPE_TEXT.to_owned()),
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
        }
    }

    fn generation_request(request: LlmRequest) -> LlmGenerationRequest {
        LlmGenerationRequest {
            session_id: SessionId::try_new("session_test").expect("session id"),
            run_id: RunId::new(2),
            turn_id: TurnId::new(3),
            request,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn generate_passes_stored_api_key_to_client() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let api = fake_api();
        let adapter = OpenAiCompletionsLlmAdapter::new(api.clone(), blobs)
            .with_provider_key_resolver(Arc::new(
                crate::provider_keys::StaticProviderKeys::new().with_key("openai", "stored-key"),
            ));

        adapter
            .generate(generation_request(request(Vec::new())))
            .await
            .expect("generate");

        assert_eq!(
            api.seen_auth.lock().expect("lock").as_slice(),
            [Some("api_key:stored-key".to_owned())]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn generate_passes_stored_oauth_bearer_to_client() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let api = fake_api();
        let adapter = OpenAiCompletionsLlmAdapter::new(api.clone(), blobs)
            .with_provider_key_resolver(Arc::new(
                crate::provider_keys::StaticProviderKeys::new()
                    .with_bearer("openai", "oauth-token"),
            ));

        adapter
            .generate(generation_request(request(Vec::new())))
            .await
            .expect("generate");

        assert_eq!(
            api.seen_auth.lock().expect("lock").as_slice(),
            [Some("bearer:oauth-token".to_owned())]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn materializes_developer_user_params_and_neutral_fields() {
        let blobs = InMemoryBlobStore::new();
        let instructions = blobs.insert_text("Be precise.").await;
        let user = blobs.insert_text("Hello").await;
        let mut request = request(vec![
            entry(
                1,
                ContextEntryKind::Instructions,
                ContextEntrySource::ContextEdit,
                instructions,
            ),
            entry(
                2,
                ContextEntryKind::Message {
                    role: ContextMessageRole::User,
                },
                ContextEntrySource::RunInput {
                    run_id: RunId::new(2),
                    input_index: 0,
                },
                user,
            ),
        ]);
        request.output_limit = Some(321);
        request.reasoning_effort = Some("max".to_owned());
        request.parallel_tool_use = Some(false);
        request.tool_choice = Some(ToolChoice::RequiredAny);
        request.params = Some(ProviderParams::new(
            ProviderApiKind::OpenAiCompletions,
            json!({
                "temperature": 0.25,
                "top_p": 0.9,
                "store": true,
                "metadata": { "suite": "unit" },
                "extra": { "extra_wire_field": "kept" }
            }),
        ));

        let materialized = materialize_create_request(&blobs, &request)
            .await
            .expect("materialize");
        let value = serde_json::to_value(materialized).expect("json");

        assert_eq!(
            value["messages"],
            json!([
                { "role": "developer", "content": "Be precise." },
                { "role": "user", "content": "Hello" }
            ])
        );
        assert_eq!(value["max_completion_tokens"], 321);
        assert_eq!(value["reasoning_effort"], "max");
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["tool_choice"], "required");
        assert_eq!(value["temperature"], 0.25);
        assert_eq!(value["extra_wire_field"], "kept");
        assert_eq!(value["stream"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn folds_images_and_pdfs_into_one_user_message() {
        let blobs = InMemoryBlobStore::new();
        let image_ref = blobs.put_bytes(vec![1, 2, 3]).await.expect("store image");
        let pdf_ref = blobs
            .put_bytes(b"%PDF-test".to_vec())
            .await
            .expect("store PDF");
        let source = ContextEntrySource::RunInput {
            run_id: RunId::new(1),
            input_index: 0,
        };
        let mut image = entry(
            1,
            ContextEntryKind::Message {
                role: ContextMessageRole::User,
            },
            source.clone(),
            image_ref,
        );
        image.media_type = Some("image/png".to_owned());
        let mut pdf = entry(
            2,
            ContextEntryKind::Message {
                role: ContextMessageRole::User,
            },
            source,
            pdf_ref,
        );
        pdf.media_type = Some("application/pdf".to_owned());
        pdf.preview = Some("[document: brief.pdf]".to_owned());

        let value = serde_json::to_value(
            materialize_create_request(&blobs, &request(vec![image, pdf]))
                .await
                .expect("materialize"),
        )
        .expect("json");

        let messages = value["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let parts = messages[0]["content"].as_array().expect("parts");
        assert_eq!(parts[0]["type"], "image_url");
        assert_eq!(parts[0]["image_url"]["url"], "data:image/png;base64,AQID");
        assert_eq!(parts[1]["type"], "file");
        assert_eq!(parts[1]["file"]["filename"], "brief.pdf");
        assert!(
            parts[1]["file"]["file_data"]
                .as_str()
                .expect("file data")
                .starts_with("data:application/pdf;base64,")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn folds_assistant_tool_calls_and_materializes_tool_results() {
        let blobs = InMemoryBlobStore::new();
        let assistant_ref = blobs.insert_text("Checking").await;
        let raw_call = json!({
            "id": "call_1",
            "type": "function",
            "function": { "name": "read_file", "arguments": "{\"path\":\"README.md\"}" }
        });
        let call_ref = put_json(&blobs, &raw_call).await.expect("call");
        let result_ref = blobs.insert_text("contents").await;
        let assistant_source = ContextEntrySource::AssistantOutput {
            run_id: RunId::new(1),
            turn_id: TurnId::new(1),
        };
        let mut call = entry(
            2,
            ContextEntryKind::ToolCall {
                call_id: ToolCallId::try_new("call_1").expect("call id"),
                name: ToolName::try_new("read_file").expect("tool name"),
            },
            assistant_source.clone(),
            call_ref,
        );
        call.media_type = Some(MEDIA_TYPE_JSON.to_owned());
        call.provider_kind = Some(OPENAI_COMPLETIONS_TOOL_CALL_PROVIDER_KIND.to_owned());
        let entries = vec![
            entry(
                1,
                ContextEntryKind::Message {
                    role: ContextMessageRole::Assistant,
                },
                assistant_source,
                assistant_ref,
            ),
            call,
            entry(
                3,
                ContextEntryKind::ToolResult {
                    call_id: ToolCallId::try_new("call_1").expect("call id"),
                    is_error: false,
                },
                ContextEntrySource::Tool {
                    run_id: RunId::new(1),
                    turn_id: TurnId::new(1),
                    batch_id: None,
                },
                result_ref,
            ),
        ];

        let value = serde_json::to_value(
            materialize_create_request(&blobs, &request(entries))
                .await
                .expect("materialize"),
        )
        .expect("json");

        assert_eq!(value["messages"].as_array().expect("messages").len(), 2);
        assert_eq!(value["messages"][0]["role"], "assistant");
        assert_eq!(value["messages"][0]["content"], "Checking");
        assert_eq!(value["messages"][0]["tool_calls"][0], raw_call);
        assert_eq!(
            value["messages"][1],
            json!({ "role": "tool", "content": "contents", "tool_call_id": "call_1" })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_foreign_provider_native_context() {
        let blobs = InMemoryBlobStore::new();
        let content_ref = put_json(&blobs, &json!({ "role": "assistant" }))
            .await
            .expect("content");
        let mut native = entry(
            1,
            ContextEntryKind::ProviderOpaque,
            ContextEntrySource::AssistantOutput {
                run_id: RunId::new(1),
                turn_id: TurnId::new(1),
            },
            content_ref,
        );
        native.media_type = Some(MEDIA_TYPE_JSON.to_owned());
        native.provider_kind = Some("openai.responses.message".to_owned());

        let error = materialize_create_request(&blobs, &request(vec![native]))
            .await
            .expect_err("foreign native context must fail");

        assert!(matches!(error, LlmAdapterError::RequestKindMismatch { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn maps_refusal_tool_call_usage_and_finish_facts() {
        let blobs = InMemoryBlobStore::new();
        let raw = json!({
            "id": "chatcmpl_1",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "refusal": "I cannot do that.",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "safe_tool", "arguments": "not-json" }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "total_tokens": 14,
                "completion_tokens_details": { "reasoning_tokens": 2 }
            }
        });
        let parsed: oai_c::Completion = serde_json::from_value(raw.clone()).expect("completion");
        let response = ApiResponse {
            parsed,
            raw_json: raw,
            status: 200,
            headers: HeaderSnapshot::default(),
        };

        let result =
            result_from_response(&blobs, &generation_request(request(Vec::new())), &response)
                .await
                .expect("result");

        assert_eq!(result.facts.finish, LlmFinish::ToolCalls);
        assert_eq!(
            result.facts.provider_response_id.as_deref(),
            Some("chatcmpl_1")
        );
        assert_eq!(
            result
                .facts
                .usage
                .as_ref()
                .and_then(|usage| usage.input_tokens),
            Some(10)
        );
        assert_eq!(
            result
                .facts
                .usage
                .as_ref()
                .and_then(|usage| usage.reasoning_tokens),
            Some(2)
        );
        assert_eq!(result.context_entries.len(), 2);
        assert_eq!(
            result.context_entries[0].provider_kind.as_deref(),
            Some(OPENAI_COMPLETIONS_REFUSAL_PROVIDER_KIND)
        );
        assert_eq!(result.facts.tool_calls.len(), 1);
        let arguments = read_json(&blobs, &result.facts.tool_calls[0].arguments_ref)
            .await
            .expect("arguments");
        assert_eq!(arguments, json!({ "__raw": "not-json" }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn compaction_uses_target_budget_and_returns_recognized_summary() {
        let blobs = InMemoryBlobStore::new();
        let user_ref = blobs.insert_text("Long conversation").await;
        let task = ContextCompactionTask {
            model: model(),
            request_fingerprint: "sha256:compact".to_owned(),
            context: request(vec![entry(
                1,
                ContextEntryKind::Message {
                    role: ContextMessageRole::User,
                },
                ContextEntrySource::ContextEdit,
                user_ref,
            )])
            .context,
            target_tokens: Some(256),
            params: None,
        };
        let materialized = materialize_compact_request(&blobs, &task)
            .await
            .expect("materialize compaction");
        assert_eq!(materialized.max_completion_tokens, Some(256));
        assert_eq!(
            materialized.messages.last().expect("instruction").role,
            "user"
        );

        let raw = json!({
            "id": "chatcmpl_compact",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "Useful summary" }
            }]
        });
        let response = ApiResponse {
            parsed: serde_json::from_value(raw.clone()).expect("response"),
            raw_json: raw,
            status: 200,
            headers: HeaderSnapshot::default(),
        };
        let compact_request = ContextCompactionRequest {
            session_id: SessionId::try_new("session_test").expect("session id"),
            request: task,
        };
        let result = result_from_compact_response(&blobs, &compact_request, &response)
            .await
            .expect("result");

        assert_eq!(result.status, ContextCompactionStatus::Succeeded);
        assert_eq!(
            result.context_entries[0].provider_kind.as_deref(),
            Some(OPENAI_COMPLETIONS_COMPACTION_PROVIDER_KIND)
        );
        assert!(matches!(
            result.context_entries[0].kind,
            ContextEntryKind::Message {
                role: ContextMessageRole::User
            }
        ));
    }
}
