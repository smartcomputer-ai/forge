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

fn live_tests_enabled(flag_name: &str) -> bool {
    match std::env::var(flag_name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
        Err(_) => false,
    }
}

fn openai_live_model() -> String {
    std::env::var("OPENAI_LIVE_MODEL").unwrap_or_else(|_| "gpt-5.2-codex".to_string())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires RUN_LIVE_ATTRACTOR_TESTS=1 and OPENAI_API_KEY (env or .env)"]
async fn attractor_live_codergen_smoke_expected_file_side_effect() {
    load_env_files();
    if !live_tests_enabled("RUN_LIVE_ATTRACTOR_TESTS") {
        return;
    }
    if std::env::var("OPENAI_API_KEY").is_err() {
        return;
    }

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
