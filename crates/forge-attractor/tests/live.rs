use forge_agent::{LocalExecutionEnvironment, OpenAiProviderProfile, Session, SessionConfig};
use forge_attractor::forge_agent::{ForgeAgentCodergenAdapter, ForgeAgentSessionBackend};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::{PipelineRunner, PipelineStatus, RunConfig, parse_dot};
use forge_llm::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};

fn load_env_files() {
    let _ = dotenvy::from_filename(".env.local");
    let _ = dotenvy::from_filename(".env");
}

fn openai_live_model() -> String {
    std::env::var("OPENAI_LIVE_MODEL").unwrap_or_else(|_| "gpt-5.2-codex".to_string())
}

/// Runs a real pipeline with an OpenAI-backed agent that creates a file.
/// Requires: OPENAI_API_KEY set in environment or .env file.
/// Run with: cargo test -p forge-attractor --test live -- --ignored
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires OPENAI_API_KEY (costs real money)"]
async fn attractor_live_codergen_smoke_expected_file_side_effect() {
    load_env_files();
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set to run this test");
    assert!(
        !api_key.trim().is_empty(),
        "OPENAI_API_KEY is set but empty"
    );

    let model = openai_live_model();
    let provider_profile = Arc::new(OpenAiProviderProfile::with_default_tools(model));
    let llm_client = Arc::new(
        Client::from_env().expect("live test requires forge-llm client env configuration"),
    );

    let workspace = TempDir::new().expect("temp workspace should create");
    let execution_env = Arc::new(LocalExecutionEnvironment::new(workspace.path()));
    let session = Session::new(
        provider_profile,
        execution_env,
        llm_client,
        SessionConfig::default(),
    )
    .expect("session should initialize");

    let backend =
        ForgeAgentSessionBackend::new(ForgeAgentCodergenAdapter::default(), Box::new(session));
    let executor = Arc::new(RegistryNodeExecutor::new(
        forge_attractor::handlers::core_registry_with_codergen_backend(Some(Arc::new(backend))),
    ));

    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [shape=box, prompt="Use tools to create live_attractor.txt with exactly one line: live-ok. Reply only DONE once the file exists."]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
    )
    .expect("graph should parse");

    let run = timeout(
        Duration::from_secs(240),
        PipelineRunner.run(
            &graph,
            RunConfig {
                run_id: Some("live-attractor-smoke".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        ),
    )
    .await
    .expect("live attractor test timed out")
    .expect("live attractor run should succeed");

    assert_eq!(run.status, PipelineStatus::Success);
    let content = std::fs::read_to_string(workspace.path().join("live_attractor.txt"))
        .expect("live_attractor.txt should be created by codergen tool calls");
    assert_eq!(content.trim(), "live-ok");
}
