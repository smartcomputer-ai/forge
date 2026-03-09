//! End-to-end tests for the CLI agent providers.
//!
//! These tests spawn real CLI processes (claude, codex, gemini) and verify that
//! the adapters correctly parse their output and return coherent results.
//! They require the CLIs to be installed and authenticated (via OAuth or API key).
//!
//! Binary paths default to `~/.local/bin/{claude,codex,gemini}`. Override with
//! `FORGE_CLAUDE_BIN`, `FORGE_CODEX_BIN`, `FORGE_GEMINI_BIN` env vars.

use forge_llm::agent_provider::{AgentLoopEvent, AgentProvider, AgentRunOptions};
use forge_llm::cli_adapters::claude_code::ClaudeCodeAgentProvider;
use forge_llm::cli_adapters::codex::CodexAgentProvider;
use forge_llm::cli_adapters::gemini::GeminiAgentProvider;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home/ubuntu".to_string()))
}

fn resolve_bin(env_var: &str, default_name: &str) -> String {
    let path = std::env::var(env_var)
        .unwrap_or_else(|_| home_dir().join(".local/bin").join(default_name).to_string_lossy().to_string());
    assert!(
        std::path::Path::new(&path).exists(),
        "CLI binary not found at '{path}'. Install it or set {env_var} to the correct path."
    );
    path
}

fn claude_bin() -> String {
    resolve_bin("FORGE_CLAUDE_BIN", "claude")
}

fn codex_bin() -> String {
    resolve_bin("FORGE_CODEX_BIN", "codex")
}

fn gemini_bin() -> String {
    resolve_bin("FORGE_GEMINI_BIN", "gemini")
}

fn default_options() -> AgentRunOptions {
    AgentRunOptions {
        working_directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        max_turns: Some(2),
        ..Default::default()
    }
}

fn options_with_event_collector() -> (AgentRunOptions, Arc<Mutex<Vec<String>>>) {
    let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let mut opts = default_options();
    opts.on_event = Some(Arc::new(move |event: AgentLoopEvent| {
        let label = match &event {
            AgentLoopEvent::TextDelta { .. } => "TextDelta".to_string(),
            AgentLoopEvent::ToolCallStart { tool_name, .. } => {
                format!("ToolCallStart({})", tool_name)
            }
            AgentLoopEvent::ToolCallEnd { call_id, .. } => {
                format!("ToolCallEnd({})", call_id)
            }
            AgentLoopEvent::Warning { message } => format!("Warning({})", message),
        };
        events_clone.lock().unwrap().push(label);
    }));
    (opts, events)
}

/// Simple prompt that requires no tools — just a text answer.
const SIMPLE_PROMPT: &str = "What is 2+2? Reply with ONLY the number, nothing else.";

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn claude_code_simple_text_response() {
    let provider = ClaudeCodeAgentProvider::new(claude_bin());
    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &default_options())
        .await
        .expect("claude-code run_to_completion should succeed");

    assert!(
        !result.text.is_empty(),
        "expected non-empty text response from claude-code"
    );
    assert!(
        result.text.contains('4'),
        "expected '4' in response, got: {}",
        result.text
    );
    assert_eq!(result.provider, "claude-code");
    assert!(!result.id.is_empty(), "expected non-empty run id");
    assert!(result.duration_ms.unwrap_or(0) > 0, "expected positive duration");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn claude_code_emits_events() {
    let provider = ClaudeCodeAgentProvider::new(claude_bin());
    let (opts, events) = options_with_event_collector();

    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &opts)
        .await
        .expect("claude-code run_to_completion should succeed");

    assert!(!result.text.is_empty());
    let collected = events.lock().unwrap();
    assert!(
        collected.iter().any(|e| e.starts_with("TextDelta")),
        "expected at least one TextDelta event, got: {:?}",
        *collected
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn claude_code_reports_usage() {
    let provider = ClaudeCodeAgentProvider::new(claude_bin());
    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &default_options())
        .await
        .expect("claude-code run_to_completion should succeed");

    // Claude Code always reports usage in stream-json mode.
    assert!(
        result.usage.total_tokens > 0,
        "expected non-zero usage, got: {:?}",
        result.usage
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn claude_code_reports_cost() {
    let provider = ClaudeCodeAgentProvider::new(claude_bin());
    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &default_options())
        .await
        .expect("claude-code run_to_completion should succeed");

    assert!(
        result.cost_usd.is_some(),
        "expected cost_usd to be reported by claude-code"
    );
}

// ---------------------------------------------------------------------------
// Codex CLI
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn codex_simple_text_response() {
    let provider = CodexAgentProvider::new(codex_bin());
    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &default_options())
        .await
        .expect("codex run_to_completion should succeed");

    assert!(
        !result.text.is_empty(),
        "expected non-empty text response from codex"
    );
    assert!(
        result.text.contains('4'),
        "expected '4' in response, got: {}",
        result.text
    );
    assert_eq!(result.provider, "codex-cli");
    assert!(!result.id.is_empty());
    assert!(result.duration_ms.unwrap_or(0) > 0);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn codex_emits_events() {
    let provider = CodexAgentProvider::new(codex_bin());
    let (opts, events) = options_with_event_collector();

    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &opts)
        .await
        .expect("codex run_to_completion should succeed");

    assert!(!result.text.is_empty());
    let collected = events.lock().unwrap();
    // Codex emits TextDelta for agent_message items.
    assert!(
        collected.iter().any(|e| e.starts_with("TextDelta")),
        "expected at least one TextDelta event, got: {:?}",
        *collected
    );
}

// ---------------------------------------------------------------------------
// Gemini CLI
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn gemini_simple_text_response() {
    let provider = GeminiAgentProvider::new(gemini_bin());
    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &default_options())
        .await
        .expect("gemini run_to_completion should succeed");

    assert!(
        !result.text.is_empty(),
        "expected non-empty text response from gemini"
    );
    assert!(
        result.text.contains('4'),
        "expected '4' in response, got: {}",
        result.text
    );
    assert_eq!(result.provider, "gemini-cli");
    assert!(!result.id.is_empty());
    assert!(result.duration_ms.unwrap_or(0) > 0);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn gemini_emits_events() {
    let provider = GeminiAgentProvider::new(gemini_bin());
    let (opts, events) = options_with_event_collector();

    let result = provider
        .run_to_completion(SIMPLE_PROMPT, &opts)
        .await
        .expect("gemini run_to_completion should succeed");

    assert!(!result.text.is_empty());
    let collected = events.lock().unwrap();
    assert!(
        collected.iter().any(|e| e.starts_with("TextDelta")),
        "expected at least one TextDelta event, got: {:?}",
        *collected
    );
}

// ---------------------------------------------------------------------------
// Cross-provider: all three produce structurally valid results
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "spawns real CLI agents (slow, requires authentication)"]
async fn all_providers_return_valid_agent_run_result() {
    let providers: Vec<Box<dyn AgentProvider>> = vec![
        Box::new(ClaudeCodeAgentProvider::new(claude_bin())),
        Box::new(CodexAgentProvider::new(codex_bin())),
        Box::new(GeminiAgentProvider::new(gemini_bin())),
    ];

    for provider in &providers {
        let result = provider
            .run_to_completion(SIMPLE_PROMPT, &default_options())
            .await
            .unwrap_or_else(|e| {
                panic!("{} failed: {}", provider.name(), e);
            });

        assert!(
            !result.text.is_empty(),
            "{}: expected non-empty text",
            provider.name()
        );
        assert!(
            !result.provider.is_empty(),
            "{}: expected non-empty provider",
            provider.name()
        );
        assert!(
            !result.model.is_empty(),
            "{}: expected non-empty model",
            provider.name()
        );
        assert!(
            result.duration_ms.unwrap_or(0) > 0,
            "{}: expected positive duration",
            provider.name()
        );
    }
}
