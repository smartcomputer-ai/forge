use forge_attractor::handlers::parallel::ParallelHandler;
use forge_attractor::handlers::parallel_fan_in::ParallelFanInHandler;
use forge_attractor::{NodeHandler, NodeStatus, RuntimeContext, parse_dot};
use serde_json::{Value, json};

#[tokio::test(flavor = "current_thread")]
async fn parallel_join_policies_expected_deterministic_outcomes() {
    let graph = parse_dot(
        r#"
        digraph G {
            p_all [shape=component, join_policy="all_success"]
            p_any [shape=component, join_policy="any_success"]
            p_quorum [shape=component, join_policy="quorum", quorum_count=2]
            p_ignore [shape=component, join_policy="ignore"]
            p_all -> a
            p_all -> b
            p_any -> a
            p_any -> b
            p_quorum -> a
            p_quorum -> b
            p_quorum -> c
            p_ignore -> a
            p_ignore -> b
        }
        "#,
    )
    .expect("graph should parse");

    let mut context = RuntimeContext::new();
    context.insert(
        "parallel.branch_outcomes".to_string(),
        json!({
            "a": "success",
            "b": "fail",
            "c": "success"
        }),
    );

    let all_outcome = ParallelHandler
        .execute(
            graph.nodes.get("p_all").expect("node should exist"),
            &context,
            &graph,
        )
        .await
        .expect("all policy should execute");
    assert_eq!(all_outcome.status, NodeStatus::Fail);

    let any_outcome = ParallelHandler
        .execute(
            graph.nodes.get("p_any").expect("node should exist"),
            &context,
            &graph,
        )
        .await
        .expect("any policy should execute");
    assert_eq!(any_outcome.status, NodeStatus::Success);

    let quorum_outcome = ParallelHandler
        .execute(
            graph.nodes.get("p_quorum").expect("node should exist"),
            &context,
            &graph,
        )
        .await
        .expect("quorum policy should execute");
    assert_eq!(quorum_outcome.status, NodeStatus::Success);

    let ignore_outcome = ParallelHandler
        .execute(
            graph.nodes.get("p_ignore").expect("node should exist"),
            &context,
            &graph,
        )
        .await
        .expect("ignore policy should execute");
    assert_eq!(ignore_outcome.status, NodeStatus::Success);
}

#[tokio::test(flavor = "current_thread")]
async fn parallel_fan_in_aggregation_expected_best_candidate_selected() {
    let graph = parse_dot("digraph G { fan [shape=tripleoctagon] }").expect("graph parse");
    let mut context = RuntimeContext::new();
    context.insert(
        "parallel.results".to_string(),
        json!([
            {"branch_id": "a", "status": "partial_success", "score": 0.2},
            {"branch_id": "b", "status": "success", "score": 0.5},
            {"branch_id": "c", "status": "success", "score": 0.9}
        ]),
    );

    let outcome = ParallelFanInHandler
        .execute(
            graph.nodes.get("fan").expect("node should exist"),
            &context,
            &graph,
        )
        .await
        .expect("fan-in should execute");

    assert_eq!(outcome.status, NodeStatus::Success);
    assert_eq!(
        outcome.context_updates.get("parallel.fan_in.best_id"),
        Some(&Value::String("c".to_string()))
    );
    assert_eq!(
        outcome.context_updates.get("parallel.fan_in.best_outcome"),
        Some(&Value::String("success".to_string()))
    );
}
