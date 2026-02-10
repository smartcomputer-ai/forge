use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::handlers::wait_human::{
    CallbackInterviewer, HumanAnswer, QueueInterviewer, WaitHumanHandler,
};
use forge_attractor::{PipelineRunner, PipelineStatus, RunConfig, parse_dot};
use std::sync::Arc;

fn gate_graph() -> forge_attractor::Graph {
    parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon, label="Review"]
            approve
            revise
            exit [shape=Msquare]
            start -> gate
            gate -> approve [label="[A] Approve"]
            gate -> revise [label="[R] Revise"]
            approve -> exit
            revise -> exit
        }
        "#,
    )
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn hitl_queue_interviewer_expected_selected_branch() {
    let graph = gate_graph();
    let interviewer = Arc::new(QueueInterviewer::with_answers(vec![HumanAnswer::Selected(
        "R".to_string(),
    )]));
    let mut registry = forge_attractor::handlers::core_registry();
    registry.register_type("wait.human", Arc::new(WaitHumanHandler::new(interviewer)));

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                executor: Arc::new(RegistryNodeExecutor::new(registry)),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|node| node == "revise"));
    assert!(!result.completed_nodes.iter().any(|node| node == "approve"));
}

#[tokio::test(flavor = "current_thread")]
async fn hitl_callback_interviewer_expected_selected_branch() {
    let graph = gate_graph();
    let interviewer = Arc::new(CallbackInterviewer::new(|question| {
        assert_eq!(question.stage, "gate");
        HumanAnswer::Selected("A".to_string())
    }));
    let mut registry = forge_attractor::handlers::core_registry();
    registry.register_type("wait.human", Arc::new(WaitHumanHandler::new(interviewer)));

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                executor: Arc::new(RegistryNodeExecutor::new(registry)),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|node| node == "approve"));
    assert!(!result.completed_nodes.iter().any(|node| node == "revise"));
}
