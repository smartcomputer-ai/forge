//! Integration tests for AgentProvider → AgentProviderSubmitter → pipeline.
//!
//! These tests verify that the full chain from DOT pipeline through the
//! AgentProviderSubmitter adapter works correctly with mock providers.
//! No real CLI binaries or API keys needed.

use async_trait::async_trait;
use forge_attractor::agent_provider::AgentProviderSubmitter;
use forge_attractor::forge_agent::{ForgeAgentCodergenAdapter, ForgeAgentSessionBackend};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::{PipelineRunner, PipelineStatus, RunConfig, prepare_pipeline};
use forge_llm::agent_provider::{
    AgentProvider, AgentRunOptions, AgentRunResult, ToolActivityRecord,
};
use forge_llm::errors::{ErrorInfo, ProviderError, ProviderErrorKind, SDKError};
use forge_llm::types::Usage;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Mock provider that returns a canned response.
struct MockAgentProvider {
    name: &'static str,
    response: String,
    model: String,
    tool_activity: Vec<ToolActivityRecord>,
}

impl MockAgentProvider {
    fn simple(response: &str) -> Self {
        Self {
            name: "mock-agent",
            response: response.to_string(),
            model: "mock-model-v1".to_string(),
            tool_activity: Vec::new(),
        }
    }

    fn with_tools(response: &str, tools: Vec<ToolActivityRecord>) -> Self {
        Self {
            name: "mock-agent",
            response: response.to_string(),
            model: "mock-model-v1".to_string(),
            tool_activity: tools,
        }
    }
}

#[async_trait]
impl AgentProvider for MockAgentProvider {
    fn name(&self) -> &str {
        self.name
    }

    async fn run_to_completion(
        &self,
        _prompt: &str,
        _options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        Ok(AgentRunResult {
            text: self.response.clone(),
            tool_activity: self.tool_activity.clone(),
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                raw: None,
            },
            id: "mock-run-001".to_string(),
            model: self.model.clone(),
            provider: self.name.to_string(),
            cost_usd: Some(0.001),
            duration_ms: Some(42),
        })
    }
}

/// Mock provider that always fails.
struct FailingAgentProvider;

#[async_trait]
impl AgentProvider for FailingAgentProvider {
    fn name(&self) -> &str {
        "failing-agent"
    }

    async fn run_to_completion(
        &self,
        _prompt: &str,
        _options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        Err(SDKError::Provider(ProviderError {
            info: ErrorInfo::new("mock agent failure: model unavailable"),
            kind: ProviderErrorKind::Server,
            status_code: Some(500),
            error_code: None,
            provider: "failing-agent".to_string(),
            retryable: false,
            retry_after: None,
            raw: None,
        }))
    }
}

fn build_executor_with_provider(
    provider: Arc<dyn AgentProvider>,
) -> Arc<dyn forge_attractor::NodeExecutor> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let submitter = AgentProviderSubmitter::new(provider, cwd);
    let backend =
        ForgeAgentSessionBackend::new(ForgeAgentCodergenAdapter::default(), Box::new(submitter));
    let registry =
        forge_attractor::handlers::core_registry_with_codergen_backend(Some(Arc::new(backend)));
    Arc::new(RegistryNodeExecutor::new(registry))
}

// ---------------------------------------------------------------------------
// Linear pipeline with mock agent provider
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn pipeline_with_mock_agent_provider_expected_success() {
    let workspace = TempDir::new().unwrap();
    let provider = Arc::new(MockAgentProvider::simple("Task completed successfully."));
    let executor = build_executor_with_provider(provider);

    let (graph, _) = prepare_pipeline(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [shape=box, prompt="Do the thing"]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
        &[],
        &[],
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("mock-agent-test".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("pipeline should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.contains(&"work".to_string()));
}

// ---------------------------------------------------------------------------
// Multi-stage pipeline
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn pipeline_multi_stage_with_mock_agent_expected_all_nodes_complete() {
    let workspace = TempDir::new().unwrap();
    let provider = Arc::new(MockAgentProvider::simple("Done."));
    let executor = build_executor_with_provider(provider);

    let (graph, _) = prepare_pipeline(
        r#"
        digraph G {
            graph [goal="Build a widget"]
            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan: $goal"]
            implement [shape=box, prompt="Implement: $goal"]
            review [shape=box, prompt="Review the implementation"]
            exit [shape=Msquare]
            start -> plan -> implement -> review -> exit
        }
        "#,
        &[],
        &[],
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("multi-stage-mock".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("pipeline should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.contains(&"plan".to_string()));
    assert!(result.completed_nodes.contains(&"implement".to_string()));
    assert!(result.completed_nodes.contains(&"review".to_string()));
}

// ---------------------------------------------------------------------------
// Provider failure propagates as node failure
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn pipeline_with_failing_provider_expected_fail_status() {
    let workspace = TempDir::new().unwrap();
    let provider = Arc::new(FailingAgentProvider);
    let executor = build_executor_with_provider(provider);

    let (graph, _) = prepare_pipeline(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [shape=box, prompt="This will fail"]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
        &[],
        &[],
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("failing-provider-test".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("pipeline should complete (with fail status, not error)");

    assert_eq!(result.status, PipelineStatus::Fail);
}

// ---------------------------------------------------------------------------
// Tool activity is captured in the outcome
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn pipeline_with_tool_activity_expected_artifacts_written() {
    let workspace = TempDir::new().unwrap();
    let tools = vec![
        ToolActivityRecord {
            tool_name: "Read".to_string(),
            call_id: "tc_001".to_string(),
            arguments_summary: Some(r#"{"file_path":"main.rs"}"#.to_string()),
            result_summary: Some("fn main() {}".to_string()),
            is_error: false,
            duration_ms: Some(10),
        },
        ToolActivityRecord {
            tool_name: "Write".to_string(),
            call_id: "tc_002".to_string(),
            arguments_summary: Some(r#"{"file_path":"out.rs"}"#.to_string()),
            result_summary: Some("written".to_string()),
            is_error: false,
            duration_ms: Some(5),
        },
    ];
    let provider = Arc::new(MockAgentProvider::with_tools("Files updated.", tools));
    let executor = build_executor_with_provider(provider);

    let (graph, _) = prepare_pipeline(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [shape=box, prompt="Update files"]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
        &[],
        &[],
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("tool-activity-test".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("pipeline should succeed");

    assert_eq!(result.status, PipelineStatus::Success);

    // Verify status.json was written for the work node
    let status_path = workspace.path().join("work").join("status.json");
    assert!(
        status_path.exists(),
        "status.json should be written for work node"
    );
    let status: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&status_path).unwrap()).unwrap();
    assert_eq!(status["outcome"], "success");
}

// ---------------------------------------------------------------------------
// Goal variable expansion works through agent provider path
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn pipeline_goal_expansion_through_agent_provider_expected_prompt_expanded() {
    let workspace = TempDir::new().unwrap();

    // Provider that echoes the prompt it receives as the response
    struct EchoPromptProvider;

    #[async_trait]
    impl AgentProvider for EchoPromptProvider {
        fn name(&self) -> &str {
            "echo-prompt"
        }

        async fn run_to_completion(
            &self,
            prompt: &str,
            _options: &AgentRunOptions,
        ) -> Result<AgentRunResult, SDKError> {
            Ok(AgentRunResult {
                text: format!("RECEIVED: {}", prompt),
                tool_activity: Vec::new(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    raw: None,
                },
                id: "echo-001".to_string(),
                model: "echo-v1".to_string(),
                provider: "echo-prompt".to_string(),
                cost_usd: None,
                duration_ms: Some(1),
            })
        }
    }

    let provider: Arc<dyn AgentProvider> = Arc::new(EchoPromptProvider);
    let executor = build_executor_with_provider(provider);

    let (graph, _) = prepare_pipeline(
        r#"
        digraph G {
            graph [goal="Build auth system"]
            start [shape=Mdiamond]
            plan [shape=box, prompt="Create a plan for $goal"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
        &[],
        &[],
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("goal-expansion-test".to_string()),
                logs_root: Some(PathBuf::from(workspace.path())),
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("pipeline should succeed");

    assert_eq!(result.status, PipelineStatus::Success);

    // The prompt artifact should contain the expanded goal (variable substitution).
    let prompt_path = workspace.path().join("plan").join("prompt.md");
    assert!(
        prompt_path.exists(),
        "prompt.md should be written for plan node"
    );
    let prompt_content = std::fs::read_to_string(&prompt_path).unwrap();
    assert!(
        prompt_content.contains("Build auth system"),
        "expected expanded goal in prompt.md, got: {}",
        prompt_content
    );
    assert!(
        !prompt_content.contains("$goal"),
        "expected $goal to be expanded, but found literal $goal in prompt.md: {}",
        prompt_content
    );
}
