use async_trait::async_trait;
use forge_attractor::handlers::{
    codergen::{CodergenBackend, CodergenBackendResult, CodergenHandler},
    conditional::ConditionalHandler,
    exit::ExitHandler,
    start::StartHandler,
    tool::ToolHandler,
    wait_human::{HumanAnswer, HumanQuestion, Interviewer, WaitHumanHandler},
};
use forge_attractor::{
    NodeHandler, NodeOutcome, NodeStatus, PipelineRunner, RunConfig, RuntimeContext, parse_dot,
};
use std::sync::Arc;

struct FixedInterviewer(HumanAnswer);

#[async_trait]
impl Interviewer for FixedInterviewer {
    async fn ask(&self, _question: HumanQuestion) -> HumanAnswer {
        self.0.clone()
    }
}

struct EchoBackend;

#[async_trait]
impl CodergenBackend for EchoBackend {
    async fn run(
        &self,
        _node: &forge_attractor::Node,
        prompt: &str,
        _context: &RuntimeContext,
        _graph: &forge_attractor::Graph,
    ) -> Result<CodergenBackendResult, forge_attractor::AttractorError> {
        Ok(CodergenBackendResult::Text(format!("echo::{prompt}")))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn handlers_start_exit_conditional_expected_success() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=diamond]
            exit [shape=Msquare]
        }
        "#,
    )
    .expect("graph should parse");

    let start = graph.nodes.get("start").expect("start");
    let gate = graph.nodes.get("gate").expect("gate");
    let exit = graph.nodes.get("exit").expect("exit");

    assert_eq!(
        StartHandler
            .execute(start, &RuntimeContext::new(), &graph)
            .await
            .expect("start")
            .status,
        NodeStatus::Success
    );
    assert_eq!(
        ConditionalHandler
            .execute(gate, &RuntimeContext::new(), &graph)
            .await
            .expect("conditional")
            .status,
        NodeStatus::Success
    );
    assert_eq!(
        ExitHandler
            .execute(exit, &RuntimeContext::new(), &graph)
            .await
            .expect("exit")
            .status,
        NodeStatus::Success
    );
}

#[tokio::test(flavor = "current_thread")]
async fn handler_codergen_backend_text_expected_success_and_context_updates() {
    let graph = parse_dot(
        r#"
        digraph G {
            graph [goal="ship"]
            n1 [shape=box, prompt="do $goal"]
        }
        "#,
    )
    .expect("graph should parse");
    let node = graph.nodes.get("n1").expect("node");
    let handler = CodergenHandler::new(Some(Arc::new(EchoBackend)));
    let outcome = handler
        .execute(node, &RuntimeContext::new(), &graph)
        .await
        .expect("execution should succeed");
    assert_eq!(outcome.status, NodeStatus::Success);
    assert!(outcome.context_updates.contains_key("last_response"));
}

#[tokio::test(flavor = "current_thread")]
async fn handler_wait_human_and_tool_expected_deterministic_results() {
    let graph = parse_dot(
        r#"
        digraph G {
            gate [shape=hexagon]
            yes
            no
            tool_ok [shape=parallelogram, tool_command="echo ok"]
            tool_bad [shape=parallelogram]
            gate -> yes [label="[Y] Yes"]
            gate -> no [label="[N] No"]
        }
        "#,
    )
    .expect("graph should parse");

    let gate = graph.nodes.get("gate").expect("gate");
    let wait_handler = WaitHumanHandler::new(Arc::new(FixedInterviewer(HumanAnswer::Selected(
        "N".to_string(),
    ))));
    let wait_outcome = wait_handler
        .execute(gate, &RuntimeContext::new(), &graph)
        .await
        .expect("wait handler");
    assert_eq!(wait_outcome.status, NodeStatus::Success);
    assert_eq!(wait_outcome.suggested_next_ids, vec!["no".to_string()]);

    let tool_ok = graph.nodes.get("tool_ok").expect("tool_ok");
    let tool_bad = graph.nodes.get("tool_bad").expect("tool_bad");
    assert_eq!(
        ToolHandler
            .execute(tool_ok, &RuntimeContext::new(), &graph)
            .await
            .expect("tool ok")
            .status,
        NodeStatus::Success
    );
    assert_eq!(
        ToolHandler
            .execute(tool_bad, &RuntimeContext::new(), &graph)
            .await
            .expect("tool bad")
            .status,
        NodeStatus::Fail
    );
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_default_registry_human_gate_expected_first_branch_auto_selected() {
    let graph = parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon]
            first
            second
            exit [shape=Msquare]
            start -> gate
            gate -> first [label="[A] First"]
            gate -> second [label="[B] Second"]
            first -> exit
            second -> exit
        }
        "#,
    )
    .expect("graph should parse");

    let result = PipelineRunner
        .run(&graph, RunConfig::default())
        .await
        .expect("run should succeed");

    assert_eq!(result.status, forge_attractor::PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|n| n == "first"));
    assert!(!result.completed_nodes.iter().any(|n| n == "second"));
}

#[tokio::test(flavor = "current_thread")]
async fn codergen_backend_outcome_passthrough_expected_fail_status() {
    struct OutcomeBackend;
    #[async_trait]
    impl CodergenBackend for OutcomeBackend {
        async fn run(
            &self,
            _node: &forge_attractor::Node,
            _prompt: &str,
            _context: &RuntimeContext,
            _graph: &forge_attractor::Graph,
        ) -> Result<CodergenBackendResult, forge_attractor::AttractorError> {
            Ok(CodergenBackendResult::Outcome(NodeOutcome::failure(
                "backend fail",
            )))
        }
    }

    let graph = parse_dot("digraph G { n1 [shape=box, label=\"n1\"] }").expect("graph parse");
    let node = graph.nodes.get("n1").expect("node");
    let handler = CodergenHandler::new(Some(Arc::new(OutcomeBackend)));
    let outcome = handler
        .execute(node, &RuntimeContext::new(), &graph)
        .await
        .expect("execute");
    assert_eq!(outcome.status, NodeStatus::Fail);
}
