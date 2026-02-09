#![allow(dead_code)]

use async_trait::async_trait;
use forge_agent::{
    AnthropicProviderProfile, AssistantTurn, GeminiProviderProfile, OpenAiProviderProfile,
    ProviderProfile, ToolResultTurn, ToolResultsTurn, Turn,
};
use forge_llm::{
    Client, ConfigurationError, ContentPart, FinishReason, Message, ProviderAdapter, Request,
    Response, SDKError, StreamEventStream, ToolCallData, Usage,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SequenceAdapter {
    pub name: String,
    pub responses: Arc<Mutex<VecDeque<Response>>>,
    pub requests: Arc<Mutex<Vec<Request>>>,
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
pub enum FixtureKind {
    OpenAi,
    Anthropic,
    Gemini,
}

impl FixtureKind {
    pub fn id(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    pub fn model(&self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-5.2-codex",
            Self::Anthropic => "claude-sonnet-4.5",
            Self::Gemini => "gemini-2.5-pro",
        }
    }

    pub fn profile(&self) -> Arc<dyn ProviderProfile> {
        match self {
            Self::OpenAi => Arc::new(OpenAiProviderProfile::with_default_tools(self.model())),
            Self::Anthropic => Arc::new(AnthropicProviderProfile::with_default_tools(self.model())),
            Self::Gemini => Arc::new(GeminiProviderProfile::with_default_tools(self.model())),
        }
    }

    pub fn edit_tool_name(&self) -> &'static str {
        match self {
            Self::OpenAi => "apply_patch",
            Self::Anthropic | Self::Gemini => "edit_file",
        }
    }
}

pub fn all_fixtures() -> [FixtureKind; 3] {
    [FixtureKind::OpenAi, FixtureKind::Anthropic, FixtureKind::Gemini]
}

pub fn usage() -> Usage {
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

pub fn text_response(provider: &str, model: &str, id: &str, text: &str) -> Response {
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

pub fn tool_call_response(
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

pub fn client_with_adapter(
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

pub fn enqueue(responses: &Arc<Mutex<VecDeque<Response>>>, response: Response) {
    responses
        .lock()
        .expect("responses mutex")
        .push_back(response);
}

pub fn tool_result_by_call_id<'a>(history: &'a [Turn], call_id: &str) -> Option<&'a ToolResultTurn> {
    history.iter().find_map(|turn| {
        if let Turn::ToolResults(ToolResultsTurn { results, .. }) = turn {
            return results.iter().find(|result| result.tool_call_id == call_id);
        }
        None
    })
}

pub fn last_assistant_text(history: &[Turn]) -> Option<String> {
    history.iter().rev().find_map(|turn| {
        if let Turn::Assistant(AssistantTurn { content, .. }) = turn {
            Some(content.clone())
        } else {
            None
        }
    })
}
