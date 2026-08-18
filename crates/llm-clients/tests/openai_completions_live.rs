use std::collections::BTreeMap;

use llm_clients::ProviderFailureKind;
use llm_clients::openai::completions::{
    API_KIND, Client, CompletionContent, CompletionMessage, CompletionMessageContent,
    CompletionTool, CompletionToolChoice, CompletionToolChoiceFunction, CompletionToolChoiceMode,
    CompletionToolType, Config, CreateCompletionRequest,
};
use serde_json::json;

mod support;

use support::{
    env_or_dotenv_var, openai_completions_create, openai_completions_stream,
    required_first_env_or_dotenv_var,
};

fn live_model() -> String {
    env_or_dotenv_var("OPENAI_COMPLETIONS_MODEL")
        .or_else(|_| env_or_dotenv_var("OPENAI_LIVE_MODEL"))
        .unwrap_or_else(|_| "gpt-5.5".to_string())
}

fn live_client() -> Client {
    let api_key = required_first_env_or_dotenv_var(
        &["OPENAI_COMPLETIONS_API_KEY", "OPENAI_API_KEY"],
        "OPENAI_COMPLETIONS_API_KEY or OPENAI_API_KEY must be set in env or root .env to run openai:completions live tests",
    );

    let mut config = Config::new(api_key);
    if let Ok(base_url) = env_or_dotenv_var("OPENAI_COMPLETIONS_BASE_URL")
        .or_else(|_| env_or_dotenv_var("OPENAI_BASE_URL"))
    {
        config.base_url = base_url;
    }
    if let Ok(org_id) = env_or_dotenv_var("OPENAI_ORG_ID") {
        config.organization = Some(org_id);
    }
    if let Ok(project) = env_or_dotenv_var("OPENAI_PROJECT_ID") {
        config.project = Some(project);
    }

    Client::new(config).expect("OpenAI completions client")
}

fn deepseek_model() -> String {
    env_or_dotenv_var("DEEPSEEK_COMPLETIONS_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".to_owned())
}

fn deepseek_client() -> Client {
    let api_key = required_first_env_or_dotenv_var(
        &["DEEPSEEK_API_KEY"],
        "DEEPSEEK_API_KEY must be set in env or root .env to run DeepSeek completions live tests",
    );
    let mut config = Config::new(api_key);
    config.base_url = env_or_dotenv_var("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com".to_owned());
    Client::new(config).expect("DeepSeek completions client")
}

fn live_api_key() -> String {
    required_first_env_or_dotenv_var(
        &["OPENAI_COMPLETIONS_API_KEY", "OPENAI_API_KEY"],
        "OPENAI_COMPLETIONS_API_KEY or OPENAI_API_KEY must be set in env or root .env to run openai:completions live tests",
    )
}

fn part(kind: &str, field: &str, value: serde_json::Value) -> CompletionContent {
    let mut extra = BTreeMap::new();
    extra.insert(field.to_owned(), value);
    CompletionContent {
        r#type: kind.to_owned(),
        extra,
        ..Default::default()
    }
}

/// 32x32 solid red PNG.
const RED_PNG_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAAKElEQVR4nO3NsQ0AAAzCMP5/un0CNkuZ41wybXsHAAAAAAAAAAAAxR4yw/wuPL6QkAAAAABJRU5ErkJggg==";

fn minimal_pdf(text: &str) -> Vec<u8> {
    let content = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref_offset = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    pdf.push_str("0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len() + 1
    ));
    pdf.into_bytes()
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY or compatible endpoint credentials (costs real money)"]
async fn openai_completions_live_create_text() {
    let client = live_client();
    let request = CreateCompletionRequest::user_text(
        live_model(),
        "Reply with exactly these two words: completion transport",
    );

    let response = openai_completions_create(&client, request)
        .await
        .expect("create completion");

    assert_eq!(response.status, 200);
    assert!(!response.parsed.id.is_empty());
    assert!(
        response
            .parsed
            .output_text()
            .to_lowercase()
            .contains("completion"),
        "expected visible text output, got {:?}",
        response.parsed.choices
    );
    assert!(
        response
            .parsed
            .usage
            .as_ref()
            .and_then(|usage| usage.total_tokens)
            .unwrap_or_default()
            > 0,
        "expected usage tokens"
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_developer_instruction_role() {
    let client = live_client();
    let request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![
            CompletionMessage {
                role: "developer".to_owned(),
                content: Some(CompletionMessageContent::Text(
                    "Reply to every user message with exactly: developer role".to_owned(),
                )),
                ..Default::default()
            },
            CompletionMessage::user("What should you say?"),
        ],
        max_completion_tokens: Some(128),
        ..Default::default()
    };

    let response = openai_completions_create(&client, request)
        .await
        .expect("developer-role completion");

    assert_eq!(
        response.parsed.output_text().trim().to_lowercase(),
        "developer role"
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_lists_models_with_per_request_auth() {
    let mut config = Config::without_api_key();
    if let Ok(base_url) = env_or_dotenv_var("OPENAI_COMPLETIONS_BASE_URL")
        .or_else(|_| env_or_dotenv_var("OPENAI_BASE_URL"))
    {
        config.base_url = base_url;
    }
    let client = Client::new(config).expect("OpenAI completions client");
    let api_key = live_api_key();

    let response = client
        .list_models_with_auth(Some(llm_clients::RequestAuth::ApiKey(&api_key)))
        .await
        .expect("list models with per-request auth");

    assert_eq!(response.status, 200);
    assert!(!response.parsed.data.is_empty(), "expected provider models");
    assert!(
        response
            .parsed
            .data
            .iter()
            .any(|model| model.id == live_model()),
        "expected configured live model in GET /models response"
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_image_input() {
    let client = live_client();
    let request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![CompletionMessage {
            role: "user".to_owned(),
            content: Some(CompletionMessageContent::Parts(vec![
                part("image_url", "image_url", json!({ "url": RED_PNG_DATA_URL })),
                CompletionContent {
                    r#type: "text".to_owned(),
                    text: Some(
                        "Name the dominant color in this image using one lowercase word."
                            .to_owned(),
                    ),
                    ..Default::default()
                },
            ])),
            ..Default::default()
        }],
        max_completion_tokens: Some(256),
        ..Default::default()
    };

    let response = openai_completions_create(&client, request)
        .await
        .expect("image completion");

    assert!(
        response.parsed.output_text().to_lowercase().contains("red"),
        "expected red image description, got {:?}",
        response.parsed.choices
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_pdf_input() {
    use base64::Engine as _;

    let client = live_client();
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(minimal_pdf("The magic word is tangerine"));
    let request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![CompletionMessage {
            role: "user".to_owned(),
            content: Some(CompletionMessageContent::Parts(vec![
                part(
                    "file",
                    "file",
                    json!({
                        "filename": "magic.pdf",
                        "file_data": format!("data:application/pdf;base64,{encoded}")
                    }),
                ),
                CompletionContent {
                    r#type: "text".to_owned(),
                    text: Some("What is the magic word? Reply with one lowercase word.".to_owned()),
                    ..Default::default()
                },
            ])),
            ..Default::default()
        }],
        max_completion_tokens: Some(256),
        ..Default::default()
    };

    let response = openai_completions_create(&client, request)
        .await
        .expect("PDF completion");

    assert!(
        response
            .parsed
            .output_text()
            .to_lowercase()
            .contains("tangerine"),
        "expected document content in output, got {:?}",
        response.parsed.choices
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_json_schema_output() {
    let client = live_client();
    let mut request = CreateCompletionRequest::user_text(
        live_model(),
        "Return the city and country for Zurich in the requested JSON shape.",
    );
    request.max_completion_tokens = Some(256);
    request.response_format = Some(json!({
        "type": "json_schema",
        "json_schema": {
            "name": "location",
            "strict": true,
            "schema": {
                "type": "object",
                "properties": {
                    "city": { "type": "string" },
                    "country": { "type": "string" }
                },
                "required": ["city", "country"],
                "additionalProperties": false
            }
        }
    }));

    let response = openai_completions_create(&client, request)
        .await
        .expect("structured completion");
    let value: serde_json::Value =
        serde_json::from_str(&response.parsed.output_text()).expect("valid JSON output");

    assert_eq!(value["city"], "Zurich");
    assert!(
        value["country"] == "Switzerland" || value["country"] == "CH",
        "unexpected country value: {}",
        value["country"]
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_reasoning_effort_and_usage_details() {
    let client = live_client();
    let mut request = CreateCompletionRequest::user_text(
        live_model(),
        "Compute 37 * 19. Reply with only the integer.",
    );
    request.reasoning_effort = Some("low".to_owned());
    request.max_completion_tokens = Some(256);

    let response = openai_completions_create(&client, request)
        .await
        .expect("reasoning completion");

    assert!(response.parsed.output_text().contains("703"));
    let usage = response.parsed.usage.expect("usage");
    assert!(usage.prompt_tokens.unwrap_or_default() > 0);
    assert!(usage.completion_tokens.unwrap_or_default() > 0);
    assert!(usage.completion_tokens_details.is_some());
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY and prompt caching support (costs real money)"]
async fn openai_completions_live_reports_cached_prompt_tokens() {
    let client = live_client();
    let cacheable_prefix = "lightspeed-cache-prefix ".repeat(1_500);
    let request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![
            CompletionMessage {
                role: "developer".to_owned(),
                content: Some(CompletionMessageContent::Text(cacheable_prefix)),
                ..Default::default()
            },
            CompletionMessage::user("Reply with exactly: cache observed"),
        ],
        max_completion_tokens: Some(64),
        ..Default::default()
    };

    openai_completions_create(&client, request.clone())
        .await
        .expect("prime prompt cache");
    let cached = openai_completions_create(&client, request)
        .await
        .expect("reuse prompt cache");
    let usage = cached.parsed.usage.expect("usage");

    assert!(
        usage.cached_tokens().unwrap_or_default() >= 1_024,
        "expected a cached prompt prefix, got {usage:?}"
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY or compatible endpoint credentials (costs real money)"]
async fn openai_completions_live_stream_text() {
    let client = live_client();
    let request =
        CreateCompletionRequest::user_text(live_model(), "Reply with exactly: completion stream");
    let mut stream = openai_completions_stream(&client, request)
        .await
        .expect("stream completion");

    let mut saw_delta = false;
    let mut saw_terminal = false;
    while let Some(event) = stream.next_chunk().await.expect("stream chunk") {
        if !event.parsed.text_delta().is_empty() {
            saw_delta = true;
        }
        if event.parsed.is_terminal() {
            saw_terminal = true;
            break;
        }
    }

    assert!(saw_delta, "expected at least one text delta");
    assert!(saw_terminal, "expected terminal stream chunk");
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY or compatible endpoint credentials (costs real money)"]
async fn openai_completions_live_forced_function_call() {
    let client = live_client();
    let mut tool = CompletionTool::function(
        "get_weather",
        json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false
        }),
    );
    tool.function.description = Some("Get the current weather for a city".to_string());
    tool.function.strict = Some(true);

    let mut request = CreateCompletionRequest::user_text(
        live_model(),
        "Call get_weather for Zurich. Do not answer in natural language.",
    );
    request.tools = Some(vec![tool]);
    request.tool_choice = Some(CompletionToolChoice::Function {
        r#type: CompletionToolType::Function,
        function: CompletionToolChoiceFunction {
            name: "get_weather".to_string(),
        },
    });

    let response = openai_completions_create(&client, request)
        .await
        .expect("function call completion");
    let calls = response.parsed.tool_calls().collect::<Vec<_>>();

    assert_eq!(calls.len(), 1, "expected one forced function call");
    assert_eq!(calls[0].name, "get_weather");
    assert!(
        calls[0].arguments.contains("Zurich"),
        "expected Zurich in function arguments: {}",
        calls[0].arguments
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_tool_call_round_trip() {
    let client = live_client();
    let mut tool = CompletionTool::function(
        "lookup_temperature",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false
        }),
    );
    tool.function.strict = Some(true);
    let first_request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![CompletionMessage::user(
            "Use lookup_temperature for Zurich, then report its result.",
        )],
        tools: Some(vec![tool.clone()]),
        tool_choice: Some(CompletionToolChoice::Function {
            r#type: CompletionToolType::Function,
            function: CompletionToolChoiceFunction {
                name: "lookup_temperature".to_owned(),
            },
        }),
        max_completion_tokens: Some(256),
        ..Default::default()
    };
    let first = openai_completions_create(&client, first_request)
        .await
        .expect("first tool completion");
    let assistant = first.parsed.choices[0]
        .message
        .clone()
        .expect("assistant tool-call message");
    let call_id = assistant
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .and_then(|call| call.id.clone())
        .expect("tool call id");
    let second_request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![
            CompletionMessage::user("Use lookup_temperature for Zurich, then report its result."),
            assistant,
            CompletionMessage {
                role: "tool".to_owned(),
                content: Some(CompletionMessageContent::Text(
                    "{\"celsius\":21}".to_owned(),
                )),
                tool_call_id: Some(call_id),
                ..Default::default()
            },
        ],
        tools: Some(vec![tool]),
        tool_choice: Some(CompletionToolChoice::Mode(CompletionToolChoiceMode::Auto)),
        max_completion_tokens: Some(256),
        ..Default::default()
    };

    let second = openai_completions_create(&client, second_request)
        .await
        .expect("tool result completion");

    assert!(
        second.parsed.output_text().contains("21"),
        "expected tool result in final answer, got {:?}",
        second.parsed.choices
    );
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn openai_completions_live_parallel_function_calls() {
    let client = live_client();
    let tool = CompletionTool::function(
        "lookup_temperature",
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false
        }),
    );
    let request = CreateCompletionRequest {
        model: live_model(),
        messages: vec![CompletionMessage::user(
            "Call lookup_temperature once for Zurich and once for Tokyo. Make both calls now and do not answer in prose.",
        )],
        tools: Some(vec![tool]),
        tool_choice: Some(CompletionToolChoice::Mode(
            CompletionToolChoiceMode::Required,
        )),
        parallel_tool_calls: Some(true),
        max_completion_tokens: Some(384),
        ..Default::default()
    };

    let response = openai_completions_create(&client, request)
        .await
        .expect("parallel tool completion");
    let calls = response.parsed.tool_calls().collect::<Vec<_>>();

    assert_eq!(calls.len(), 2, "expected two tool calls, got {calls:?}");
    let arguments = calls
        .iter()
        .map(|call| call.arguments.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(arguments.contains("zurich"));
    assert!(arguments.contains("tokyo"));
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY or compatible endpoint credentials (costs real money)"]
async fn openai_completions_live_invalid_model_classifies_provider_error() {
    let client = live_client();
    let request = CreateCompletionRequest::user_text(
        "definitely-not-a-real-openai-model-for-lightspeed-tests",
        "hello",
    );

    let error = client
        .create(request)
        .await
        .expect_err("invalid model should fail");

    match error {
        llm_clients::LlmApiError::HttpStatus(provider) => {
            assert_eq!(provider.api_kind, API_KIND);
            assert!(
                matches!(
                    provider.kind,
                    ProviderFailureKind::InvalidRequest
                        | ProviderFailureKind::NotFound
                        | ProviderFailureKind::Other
                ),
                "unexpected provider failure kind: {:?}",
                provider.kind
            );
            assert!(provider.raw_json.is_some() || provider.raw_text.is_some());
        }
        other => panic!("expected provider HTTP error, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY (costs real money)"]
async fn deepseek_completions_live_v4_reasoning_and_cache_usage() {
    let client = deepseek_client();
    let request = CreateCompletionRequest {
        model: deepseek_model(),
        messages: vec![
            CompletionMessage {
                role: "system".to_owned(),
                content: Some(CompletionMessageContent::Text(
                    "Follow exact-output requests.".to_owned(),
                )),
                ..Default::default()
            },
            CompletionMessage::user("Compute 37 * 19. Reply with only the integer."),
        ],
        max_tokens: Some(256),
        reasoning_effort: Some("high".to_owned()),
        extra: BTreeMap::from([("thinking".to_owned(), json!({"type":"enabled"}))]),
        ..Default::default()
    };

    let response = openai_completions_create(&client, request)
        .await
        .expect("DeepSeek reasoning completion");
    let message = response.parsed.choices[0]
        .message
        .as_ref()
        .expect("assistant message");
    assert!(message.text().contains("703"));
    assert!(
        message
            .extra
            .get("reasoning_content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|reasoning| !reasoning.is_empty()),
        "expected DeepSeek reasoning_content, got {message:?}"
    );
    let usage = response.parsed.usage.expect("usage");
    assert!(usage.prompt_tokens.unwrap_or_default() > 0);
    assert!(
        usage.cached_tokens().is_some() || usage.cache_miss_tokens().is_some(),
        "expected DeepSeek cache accounting, got {usage:?}"
    );
}

#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY (costs real money)"]
async fn deepseek_completions_live_v4_exact_reasoning_tool_round_trip() {
    let client = deepseek_client();
    let tool = CompletionTool::function(
        "lookup_temperature",
        json!({
            "type":"object",
            "properties":{"city":{"type":"string"}},
            "required":["city"]
        }),
    );
    let user = CompletionMessage::user(
        "Call lookup_temperature for Zurich, then report the returned temperature.",
    );
    let first_request = CreateCompletionRequest {
        model: deepseek_model(),
        messages: vec![user.clone()],
        tools: Some(vec![tool.clone()]),
        // DeepSeek V4 thinking mode supports tools but rejects the
        // tool_choice parameter. With tools present, omission means auto.
        tool_choice: None,
        max_tokens: Some(512),
        reasoning_effort: Some("high".to_owned()),
        extra: BTreeMap::from([("thinking".to_owned(), json!({"type":"enabled"}))]),
        ..Default::default()
    };
    let first = openai_completions_create(&client, first_request)
        .await
        .expect("DeepSeek tool call");
    let assistant = first.parsed.choices[0]
        .message
        .clone()
        .expect("assistant tool-call message");
    assert!(
        assistant.content.is_some(),
        "DeepSeek tool-call assistant content must be non-null"
    );
    let reasoning = assistant
        .extra
        .get("reasoning_content")
        .and_then(serde_json::Value::as_str)
        .expect("reasoning_content")
        .to_owned();
    assert!(!reasoning.is_empty());
    let call_id = assistant
        .tool_calls
        .as_ref()
        .and_then(|calls| calls.first())
        .and_then(|call| call.id.clone())
        .expect("tool call id");
    let second_request = CreateCompletionRequest {
        model: deepseek_model(),
        messages: vec![
            user,
            assistant,
            CompletionMessage {
                role: "tool".to_owned(),
                content: Some(CompletionMessageContent::Text(
                    "{\"celsius\":21}".to_owned(),
                )),
                tool_call_id: Some(call_id),
                ..Default::default()
            },
        ],
        tools: Some(vec![tool]),
        tool_choice: None,
        max_tokens: Some(512),
        reasoning_effort: Some("high".to_owned()),
        extra: BTreeMap::from([("thinking".to_owned(), json!({"type":"enabled"}))]),
        ..Default::default()
    };

    let second = openai_completions_create(&client, second_request)
        .await
        .expect("DeepSeek tool result completion with reasoning replay");
    assert!(
        second.parsed.output_text().contains("21"),
        "expected tool result in final answer, got {:?}",
        second.parsed.choices
    );
}
