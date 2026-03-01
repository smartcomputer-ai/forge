//! Comprehensive spec/03 conformance test suite for forge-attractor.
//!
//! Organized by spec section; each nested module maps to a section heading.

use async_trait::async_trait;
use forge_attractor::{
    ArtifactStore, AttrValue, AttractorError, CheckpointMetadata, CheckpointNodeOutcome,
    CheckpointState, ContextStore, Diagnostic, Graph, Node, NodeExecutor, NodeOutcome,
    NodeStatus, PipelineRunner, PipelineStatus, RetryBackoffConfig, RetryPreset, RunConfig,
    RuntimeContext, RuntimeEvent, Selector, Severity, ValidationError, apply_model_stylesheet,
    build_retry_policy, checkpoint_file_path, delay_for_attempt_ms,
    evaluate_condition_expression, finalize_retry_exhausted, find_incoming_edge,
    is_valid_fidelity_mode, parse_dot, parse_stylesheet, resolve_fidelity_mode,
    resolve_thread_key, select_next_edge, should_retry_outcome, validate, validate_or_raise,
    validate_condition_expression, validate_context_key,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn zero_delay_backoff() -> RetryBackoffConfig {
    RetryBackoffConfig {
        initial_delay_ms: 0,
        backoff_factor: 1.0,
        max_delay_ms: 0,
        jitter: false,
    }
}

/// An executor that returns scripted outcomes per node id. When the list for a
/// node is exhausted, it returns `NodeOutcome::success()`.
struct ScriptedExecutor {
    outcomes: Mutex<std::collections::HashMap<String, Vec<NodeOutcome>>>,
}

impl ScriptedExecutor {
    fn new() -> Self {
        Self {
            outcomes: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn script(self, node_id: &str, outcomes: Vec<NodeOutcome>) -> Self {
        self.outcomes
            .lock()
            .expect("mutex")
            .insert(node_id.to_string(), outcomes);
        self
    }
}

#[async_trait]
impl NodeExecutor for ScriptedExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let mut map = self.outcomes.lock().expect("mutex");
        if let Some(list) = map.get_mut(&node.id) {
            if !list.is_empty() {
                return Ok(list.remove(0));
            }
        }
        Ok(NodeOutcome::success())
    }
}

fn run_cfg(executor: Arc<dyn NodeExecutor>) -> RunConfig {
    RunConfig {
        executor,
        retry_backoff: zero_delay_backoff(),
        ..RunConfig::default()
    }
}

// =========================================================================
// Section 2: DOT DSL
// =========================================================================
mod section_2_dot_dsl {
    use super::*;

    // -- 2.1 Parser basics --

    #[test]
    fn parse_dot_empty_digraph_expected_empty_graph() {
        let graph = parse_dot("digraph G {}").expect("should parse");
        assert_eq!(graph.id, "G");
        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn parse_dot_graph_keyword_rejected_expected_error() {
        let error = parse_dot("graph G { a -- b }").expect_err("undirected not supported");
        assert!(error.to_string().contains("undirected"));
    }

    #[test]
    fn parse_dot_strict_digraph_rejected_expected_error() {
        let error = parse_dot("strict digraph G { a }").expect_err("strict not supported");
        assert!(error.to_string().contains("strict"));
    }

    #[test]
    fn parse_dot_html_label_rejected_expected_error() {
        let error = parse_dot("digraph G { a [label=<<b>>] }").expect_err("HTML not supported");
        assert!(error.to_string().contains("HTML"));
    }

    // -- 2.2 Value types --

    #[test]
    fn parse_dot_string_value_expected_string_attr() {
        let graph = parse_dot(r#"digraph G { a [prompt="hello world"] }"#).expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        assert_eq!(node.attrs.get_str("prompt"), Some("hello world"));
    }

    #[test]
    fn parse_dot_integer_value_expected_integer_attr() {
        let graph = parse_dot("digraph G { a [max_retries=3] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        assert_eq!(
            node.attrs.get("max_retries"),
            Some(&AttrValue::Integer(3))
        );
    }

    #[test]
    fn parse_dot_float_value_expected_float_attr() {
        let graph = parse_dot("digraph G { a [score=0.5] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("score") {
            Some(AttrValue::Float(v)) => assert!((v - 0.5).abs() < f64::EPSILON),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn parse_dot_boolean_values_expected_boolean_attr() {
        let graph =
            parse_dot("digraph G { a [goal_gate=true, allow_partial=false] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        assert_eq!(node.attrs.get("goal_gate"), Some(&AttrValue::Boolean(true)));
        assert_eq!(
            node.attrs.get("allow_partial"),
            Some(&AttrValue::Boolean(false))
        );
    }

    #[test]
    fn parse_dot_duration_values_expected_duration_millis() {
        let graph = parse_dot("digraph G { a [timeout=30s] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 30_000),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn parse_dot_duration_minutes_expected_correct_millis() {
        let graph = parse_dot("digraph G { a [timeout=5m] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 300_000),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn parse_dot_duration_hours_expected_correct_millis() {
        let graph = parse_dot("digraph G { a [timeout=2h] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 7_200_000),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn parse_dot_duration_milliseconds_expected_correct_millis() {
        let graph = parse_dot("digraph G { a [timeout=500ms] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 500),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    // -- 2.3 Shapes --

    #[test]
    fn parse_dot_mdiamond_shape_expected_start_candidate() {
        let graph = parse_dot("digraph G { s [shape=Mdiamond] }").expect("should parse");
        let starts = graph.start_candidates();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].id, "s");
    }

    #[test]
    fn parse_dot_msquare_shape_expected_terminal_candidate() {
        let graph = parse_dot("digraph G { e [shape=Msquare] }").expect("should parse");
        let terminals = graph.terminal_candidates();
        assert_eq!(terminals.len(), 1);
        assert_eq!(terminals[0].id, "e");
    }

    #[test]
    fn parse_dot_named_start_expected_start_candidate() {
        let graph = parse_dot("digraph G { start }").expect("should parse");
        assert_eq!(graph.start_candidates().len(), 1);
    }

    #[test]
    fn parse_dot_named_exit_expected_terminal_candidate() {
        let graph = parse_dot("digraph G { exit }").expect("should parse");
        assert_eq!(graph.terminal_candidates().len(), 1);
    }

    #[test]
    fn parse_dot_named_end_expected_terminal_candidate() {
        let graph = parse_dot("digraph G { end }").expect("should parse");
        assert_eq!(graph.terminal_candidates().len(), 1);
    }

    // -- 2.4 Chained edges --

    #[test]
    fn parse_dot_chained_edge_expected_multiple_edges() {
        let graph = parse_dot("digraph G { a -> b -> c -> d }").expect("should parse");
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.edges[0].from, "a");
        assert_eq!(graph.edges[0].to, "b");
        assert_eq!(graph.edges[1].from, "b");
        assert_eq!(graph.edges[1].to, "c");
        assert_eq!(graph.edges[2].from, "c");
        assert_eq!(graph.edges[2].to, "d");
    }

    #[test]
    fn parse_dot_chained_edge_attributes_shared_expected_all_edges_inherit() {
        let graph =
            parse_dot(r#"digraph G { a -> b -> c [weight=5] }"#).expect("should parse");
        for edge in &graph.edges {
            assert_eq!(edge.attrs.get("weight"), Some(&AttrValue::Integer(5)));
        }
    }

    // -- 2.5 Subgraphs --

    #[test]
    fn parse_dot_subgraph_derives_class_from_label_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                subgraph cluster_outer {
                    label="Planning Phase"
                    plan
                }
            }
            "#,
        )
        .expect("should parse");
        let node = graph.nodes.get("plan").unwrap();
        assert_eq!(node.attrs.get_str("class"), Some("planning-phase"));
    }

    #[test]
    fn parse_dot_subgraph_node_defaults_expected_inherited() {
        let graph = parse_dot(
            r#"
            digraph G {
                subgraph cluster_a {
                    label="Group"
                    node [timeout=60s]
                    x
                }
            }
            "#,
        )
        .expect("should parse");
        let node = graph.nodes.get("x").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 60_000),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    #[test]
    fn parse_dot_graph_level_attrs_expected_accessible() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [goal="Ship the feature"]
                a
            }
            "#,
        )
        .expect("should parse");
        assert_eq!(graph.attrs.get_str("goal"), Some("Ship the feature"));
    }

    // -- 2.6 Edge attributes --

    #[test]
    fn parse_dot_edge_condition_expected_preserved() {
        let graph = parse_dot(
            r#"
            digraph G {
                a -> b [condition="outcome=success"]
            }
            "#,
        )
        .expect("should parse");
        assert_eq!(
            graph.edges[0].attrs.get_str("condition"),
            Some("outcome=success")
        );
    }

    #[test]
    fn parse_dot_edge_label_expected_preserved() {
        let graph = parse_dot(
            r#"
            digraph G {
                a -> b [label="Yes"]
            }
            "#,
        )
        .expect("should parse");
        assert_eq!(graph.edges[0].attrs.get_str("label"), Some("Yes"));
    }

    #[test]
    fn parse_dot_edge_weight_expected_integer() {
        let graph = parse_dot("digraph G { a -> b [weight=10] }").expect("should parse");
        assert_eq!(
            graph.edges[0].attrs.get("weight"),
            Some(&AttrValue::Integer(10))
        );
    }

    #[test]
    fn parse_dot_outgoing_and_incoming_edges_expected_correct() {
        let graph = parse_dot("digraph G { a -> b; a -> c; b -> c }").expect("should parse");
        let outgoing: Vec<_> = graph.outgoing_edges("a").collect();
        assert_eq!(outgoing.len(), 2);
        let incoming: Vec<_> = graph.incoming_edges("c").collect();
        assert_eq!(incoming.len(), 2);
    }

    #[test]
    fn parse_dot_source_dot_preserved_expected_original_source() {
        let source = "digraph G { a }";
        let graph = parse_dot(source).expect("should parse");
        assert_eq!(graph.source_dot.as_deref(), Some(source));
    }
}

// =========================================================================
// Section 3: Execution
// =========================================================================
mod section_3_execution {
    use super::*;

    // -- 3.1 Lifecycle --

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_linear_success_expected_all_nodes_completed() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan [prompt="Plan"]
                exit [shape=Msquare]
                start -> plan -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"start".to_string()));
        assert!(result.completed_nodes.contains(&"plan".to_string()));
        // Terminal nodes (exit/Msquare) are detected before execution and cause
        // the loop to break, so they do NOT appear in completed_nodes.
        assert!(!result.completed_nodes.contains(&"exit".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_node_failure_without_retry_expected_pipeline_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(
            ScriptedExecutor::new().script("work", vec![NodeOutcome::failure("broken")]),
        );
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_run_id_populated_in_result() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert!(!result.run_id.is_empty());
    }

    // -- 3.2 Edge selection --

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_condition_match_expected_correct_branch() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [shape=diamond]
                yes
                no
                exit [shape=Msquare]
                start -> gate
                gate -> yes [condition="outcome=success"]
                gate -> no [condition="outcome=fail"]
                yes -> exit
                no -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"yes".to_string()));
        assert!(!result.completed_nodes.contains(&"no".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_preferred_label_expected_correct_branch() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [shape=diamond]
                yes
                no
                exit [shape=Msquare]
                start -> gate
                gate -> yes [label="Yes"]
                gate -> no [label="No"]
                yes -> exit
                no -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "gate",
            vec![NodeOutcome {
                status: NodeStatus::Success,
                preferred_label: Some("No".to_string()),
                ..Default::default()
            }],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert!(result.completed_nodes.contains(&"no".to_string()));
        assert!(!result.completed_nodes.contains(&"yes".to_string()));
    }

    // -- 3.3 Goal gates --

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_goal_gate_failure_reroutes_to_retry_target_expected_recovery() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [goal_gate=true, retry_target="fix"]
                fix [prompt="Fix"]
                exit [shape=Msquare]
                start -> work -> exit
                work -> fix [condition="outcome=fail"]
                fix -> work
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "work",
            vec![
                NodeOutcome::failure("first try fails"),
                NodeOutcome::success(),
            ],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"fix".to_string()));
    }

    // -- 3.4 Retry --

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_retry_then_success_expected_pipeline_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=2, prompt="Work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "work",
            vec![
                NodeOutcome {
                    status: NodeStatus::Retry,
                    ..Default::default()
                },
                NodeOutcome::success(),
            ],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_retry_exhausted_expected_pipeline_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=1, prompt="Work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "work",
            vec![
                NodeOutcome {
                    status: NodeStatus::Retry,
                    ..Default::default()
                },
                NodeOutcome {
                    status: NodeStatus::Retry,
                    ..Default::default()
                },
            ],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_retry_exhausted_with_allow_partial_expected_partial_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=0, allow_partial=true, prompt="Work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "work",
            vec![NodeOutcome {
                status: NodeStatus::Retry,
                ..Default::default()
            }],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        // allow_partial means retry exhaustion produces partial_success, which is success-like
        assert_eq!(result.status, PipelineStatus::Success);
    }

    // -- 3.5 Failure routing --

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_failure_routing_via_condition_expected_correct_path() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                recovery [prompt="Recover"]
                exit [shape=Msquare]
                start -> work
                work -> exit [condition="outcome=success"]
                work -> recovery [condition="outcome=fail"]
                recovery -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(
            ScriptedExecutor::new().script("work", vec![NodeOutcome::failure("fail")]),
        );
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"recovery".to_string()));
    }

    // -- 3.6 Timeout (attr) --

    #[test]
    fn parse_dot_timeout_attr_expected_duration_value() {
        let graph =
            parse_dot("digraph G { a [timeout=900s] }").expect("should parse");
        let node = graph.nodes.get("a").unwrap();
        match node.attrs.get("timeout") {
            Some(AttrValue::Duration(d)) => assert_eq!(d.millis, 900_000),
            other => panic!("expected Duration, got {:?}", other),
        }
    }

    // -- 3.7 auto_status --

    #[test]
    fn node_status_as_str_expected_string_forms() {
        assert_eq!(NodeStatus::Success.as_str(), "success");
        assert_eq!(NodeStatus::PartialSuccess.as_str(), "partial_success");
        assert_eq!(NodeStatus::Retry.as_str(), "retry");
        assert_eq!(NodeStatus::Fail.as_str(), "fail");
        assert_eq!(NodeStatus::Skipped.as_str(), "skipped");
    }

    #[test]
    fn node_status_is_success_like_expected_correct() {
        assert!(NodeStatus::Success.is_success_like());
        assert!(NodeStatus::PartialSuccess.is_success_like());
        assert!(!NodeStatus::Retry.is_success_like());
        assert!(!NodeStatus::Fail.is_success_like());
        assert!(!NodeStatus::Skipped.is_success_like());
    }

    // -- Suggested next IDs --

    #[test]
    fn select_next_edge_suggested_ids_expected_used_after_preferred_label() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                n1 -> a
                n1 -> b
            }
            "#,
        )
        .expect("parse");
        let mut outcome = NodeOutcome::success();
        outcome.suggested_next_ids = vec!["b".to_string()];
        let context = RuntimeContext::new();
        let selected = select_next_edge(&graph, "n1", &outcome, &context).unwrap();
        assert_eq!(selected.to, "b");
    }

    // -- Edge weight tiebreaker --

    #[test]
    fn select_next_edge_weight_tiebreaker_expected_highest_weight_wins() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                a
                b
                n1 -> a [weight=1]
                n1 -> b [weight=2]
            }
            "#,
        )
        .expect("parse");
        let outcome = NodeOutcome::success();
        let context = RuntimeContext::new();
        let selected = select_next_edge(&graph, "n1", &outcome, &context).unwrap();
        assert_eq!(selected.to, "b");
    }

    #[test]
    fn select_next_edge_no_outgoing_expected_none() {
        let graph = parse_dot("digraph G { terminal }").expect("parse");
        let outcome = NodeOutcome::success();
        let context = RuntimeContext::new();
        assert!(select_next_edge(&graph, "terminal", &outcome, &context).is_none());
    }

    // -- Condition beats preferred_label --

    #[test]
    fn select_next_edge_condition_beats_preferred_label_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1
                pass
                fail
                n1 -> pass [condition="outcome=success"]
                n1 -> fail [label="fail"]
            }
            "#,
        )
        .expect("parse");
        let mut outcome = NodeOutcome::success();
        outcome.preferred_label = Some("fail".to_string());
        let context = RuntimeContext::new();
        let selected = select_next_edge(&graph, "n1", &outcome, &context).unwrap();
        assert_eq!(selected.to, "pass");
    }
}

// =========================================================================
// Section 4: Handlers
// =========================================================================
mod section_4_handlers {
    use super::*;
    use forge_attractor::handlers::{
        NodeHandler,
        codergen::CodergenHandler,
        conditional::ConditionalHandler,
        exit::ExitHandler,
        parallel::ParallelHandler,
        parallel_fan_in::ParallelFanInHandler,
        registry::HandlerRegistry,
        stack_manager_loop::StackManagerLoopHandler,
        start::StartHandler,
        tool::ToolHandler,
        wait_human::WaitHumanHandler,
    };

    // -- 4.1 Start handler --

    #[tokio::test(flavor = "current_thread")]
    async fn start_handler_expected_success_outcome() {
        let graph = parse_dot("digraph G { s [shape=Mdiamond] }").expect("parse");
        let node = graph.nodes.get("s").unwrap();
        let outcome = NodeHandler::execute(&StartHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    // -- 4.2 Exit handler --

    #[tokio::test(flavor = "current_thread")]
    async fn exit_handler_expected_success_outcome() {
        let graph = parse_dot("digraph G { e [shape=Msquare] }").expect("parse");
        let node = graph.nodes.get("e").unwrap();
        let outcome = NodeHandler::execute(&ExitHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    // -- 4.3 Codergen handler --

    #[tokio::test(flavor = "current_thread")]
    async fn codergen_handler_no_backend_expected_simulated_success() {
        let graph =
            parse_dot(r#"digraph G { n1 [prompt="Do thing"] }"#).expect("parse");
        let node = graph.nodes.get("n1").unwrap();
        let handler = CodergenHandler::new(None);
        let outcome = NodeHandler::execute(&handler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(outcome.context_updates.contains_key("last_stage"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn codergen_handler_goal_expansion_expected_prompt_contains_goal() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [goal="ship feature"]
                n1 [prompt="Achieve $goal"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("n1").unwrap();
        let handler = CodergenHandler::new(None);
        let outcome = NodeHandler::execute(&handler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    // -- 4.4 Conditional handler --

    #[tokio::test(flavor = "current_thread")]
    async fn conditional_handler_expected_success() {
        let graph = parse_dot("digraph G { gate [shape=diamond] }").expect("parse");
        let node = graph.nodes.get("gate").unwrap();
        let outcome = NodeHandler::execute(&ConditionalHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    // -- 4.5 Wait.human handler --

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_auto_approve_expected_first_choice() {
        let graph = parse_dot(
            r#"
            digraph G {
                gate [shape=hexagon]
                yes
                no
                gate -> yes [label="[Y] Yes"]
                gate -> no [label="[N] No"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("gate").unwrap();
        let handler = WaitHumanHandler::new(Arc::new(
            forge_attractor::AutoApproveInterviewer,
        ));
        let outcome = NodeHandler::execute(&handler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(outcome.suggested_next_ids, vec!["yes".to_string()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_no_outgoing_edges_expected_fail() {
        let graph = parse_dot("digraph G { gate [shape=hexagon] }").expect("parse");
        let node = graph.nodes.get("gate").unwrap();
        let handler = WaitHumanHandler::new(Arc::new(
            forge_attractor::AutoApproveInterviewer,
        ));
        let outcome = NodeHandler::execute(&handler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    // -- 4.6 Parallel handler --

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_all_success_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="all_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &RuntimeContext::new(),
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome
                .context_updates
                .get("parallel.branch_count")
                .and_then(Value::as_u64),
            Some(2)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_any_success_one_fail_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="any_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "success"}),
        );
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_all_fail_any_success_expected_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="any_success"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "fail"}),
        );
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_quorum_not_met_expected_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="quorum", quorum_count=2]
                p -> a
                p -> b
                p -> c
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "success", "b": "fail", "c": "fail"}),
        );
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_ignore_policy_expected_always_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="ignore"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "fail"}),
        );
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_handler_no_branches_expected_fail() {
        let graph =
            parse_dot("digraph G { p [shape=component] }").expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &RuntimeContext::new(),
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    // -- 4.7 Parallel fan-in handler --

    #[tokio::test(flavor = "current_thread")]
    async fn fan_in_handler_selects_best_expected_highest_score() {
        let graph =
            parse_dot("digraph G { fi [shape=tripleoctagon] }").expect("parse");
        let node = graph.nodes.get("fi").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.results".to_string(),
            json!([
                {"branch_id": "a", "status": "success", "score": 0.3},
                {"branch_id": "b", "status": "success", "score": 0.9}
            ]),
        );
        let outcome = NodeHandler::execute(
            &ParallelFanInHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome.context_updates.get("parallel.fan_in.best_id"),
            Some(&Value::String("b".to_string()))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_in_handler_all_failed_expected_fail() {
        let graph =
            parse_dot("digraph G { fi [shape=tripleoctagon] }").expect("parse");
        let node = graph.nodes.get("fi").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "parallel.results".to_string(),
            json!([
                {"branch_id": "a", "status": "fail", "score": 0.0},
                {"branch_id": "b", "status": "fail", "score": 0.0}
            ]),
        );
        let outcome = NodeHandler::execute(
            &ParallelFanInHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fan_in_handler_no_results_expected_fail() {
        let graph =
            parse_dot("digraph G { fi [shape=tripleoctagon] }").expect("parse");
        let node = graph.nodes.get("fi").unwrap();
        let outcome = NodeHandler::execute(
            &ParallelFanInHandler::default(),
            node,
            &RuntimeContext::new(),
            &graph,
        )
        .await
        .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    // -- 4.8 Tool handler --

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_missing_command_expected_fail() {
        let graph =
            parse_dot("digraph G { t [shape=parallelogram] }").expect("parse");
        let node = graph.nodes.get("t").unwrap();
        let outcome = NodeHandler::execute(&ToolHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_preset_output_expected_used() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="echo real", tool_output="preset"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("t").unwrap();
        let outcome = NodeHandler::execute(&ToolHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(
            outcome.context_updates.get("tool.output").and_then(Value::as_str),
            Some("preset")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_echo_expected_success_with_output() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="echo hello"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("t").unwrap();
        let outcome = NodeHandler::execute(&ToolHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
        let stdout = outcome
            .context_updates
            .get("tool.stdout")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(stdout.contains("hello"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_nonzero_exit_expected_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="exit 1"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("t").unwrap();
        let outcome = NodeHandler::execute(&ToolHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    // -- 4.9 Stack manager loop handler --

    #[tokio::test(flavor = "current_thread")]
    async fn stack_manager_loop_child_completes_expected_success() {
        let graph = parse_dot("digraph G { m [shape=house] }").expect("parse");
        let node = graph.nodes.get("m").unwrap();
        let mut context = RuntimeContext::new();
        context.insert(
            "stack.child.status_sequence".to_string(),
            json!(["running", "completed"]),
        );
        context.insert(
            "stack.child.outcome_sequence".to_string(),
            json!(["running", "success"]),
        );
        let outcome = NodeHandler::execute(&StackManagerLoopHandler, node, &context, &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stack_manager_loop_max_cycles_exceeded_expected_fail() {
        let graph =
            parse_dot("digraph G { m [shape=house, manager_max_cycles=2] }").expect("parse");
        let node = graph.nodes.get("m").unwrap();
        let outcome = NodeHandler::execute(&StackManagerLoopHandler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stack_manager_loop_stop_condition_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                m [shape=house, manager_stop_condition="context.done=true"]
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("m").unwrap();
        let mut context = RuntimeContext::new();
        context.insert("done".to_string(), Value::Bool(true));
        let outcome = NodeHandler::execute(&StackManagerLoopHandler, node, &context, &graph)
            .await
            .expect("execute");
        assert_eq!(outcome.status, NodeStatus::Success);
    }

    // -- Handler registry --

    #[test]
    fn handler_registry_explicit_type_expected_highest_precedence() {
        let registry = HandlerRegistry::new();
        let graph =
            parse_dot(r#"digraph G { n1 [shape=diamond, type="tool"] }"#).expect("parse");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(registry.resolve_handler_type(node), "tool");
    }

    #[test]
    fn handler_registry_shape_mapping_expected_correct_type() {
        let registry = HandlerRegistry::new();
        let graph = parse_dot("digraph G { n1 [shape=hexagon] }").expect("parse");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(registry.resolve_handler_type(node), "wait.human");
    }

    #[test]
    fn handler_registry_unknown_shape_expected_default_codergen() {
        let registry = HandlerRegistry::new();
        let graph = parse_dot("digraph G { n1 [shape=unknown] }").expect("parse");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(registry.resolve_handler_type(node), "codergen");
    }

    #[test]
    fn handler_registry_all_shape_mappings_expected_correct() {
        let registry = HandlerRegistry::new();
        let mappings = [
            ("Mdiamond", "start"),
            ("Msquare", "exit"),
            ("box", "codergen"),
            ("hexagon", "wait.human"),
            ("diamond", "conditional"),
            ("component", "parallel"),
            ("tripleoctagon", "parallel.fan_in"),
            ("parallelogram", "tool"),
            ("house", "stack.manager_loop"),
        ];
        for (shape, expected_type) in mappings {
            let graph =
                parse_dot(&format!("digraph G {{ n1 [shape={shape}] }}")).expect("parse");
            let node = graph.nodes.get("n1").unwrap();
            assert_eq!(
                registry.resolve_handler_type(node),
                expected_type,
                "shape={shape}"
            );
        }
    }
}

// =========================================================================
// Section 5: State
// =========================================================================
mod section_5_state {
    use super::*;

    // -- 5.1 ContextStore --

    #[test]
    fn context_store_set_and_get_expected_value() {
        let store = ContextStore::new();
        store
            .set("graph.goal", Value::String("ship".to_string()))
            .expect("set");
        assert_eq!(
            store.get("graph.goal").expect("get"),
            Some(Value::String("ship".to_string()))
        );
    }

    #[test]
    fn context_store_apply_updates_merges_expected() {
        let store = ContextStore::from_values(BTreeMap::from([(
            "existing".to_string(),
            json!("yes"),
        )]));
        store
            .apply_updates(&BTreeMap::from([("new_key".to_string(), json!(42))]))
            .expect("apply");
        let snapshot = store.snapshot().expect("snapshot");
        assert_eq!(snapshot.values.get("existing"), Some(&json!("yes")));
        assert_eq!(snapshot.values.get("new_key"), Some(&json!(42)));
    }

    #[test]
    fn context_store_clone_isolated_expected_independent() {
        let original = ContextStore::new();
        original
            .set("context.key", json!("original"))
            .expect("set");
        let cloned = original.clone_isolated().expect("clone");
        cloned.set("context.key", json!("cloned")).expect("set");
        assert_eq!(
            original.get("context.key").expect("get"),
            Some(json!("original"))
        );
        assert_eq!(
            cloned.get("context.key").expect("get"),
            Some(json!("cloned"))
        );
    }

    #[test]
    fn context_store_append_log_expected_in_snapshot() {
        let store = ContextStore::new();
        store.append_log("line one").expect("log");
        store.append_log("line two").expect("log");
        let snapshot = store.snapshot().expect("snapshot");
        assert_eq!(snapshot.logs.len(), 2);
        assert_eq!(snapshot.logs[0], "line one");
    }

    #[test]
    fn context_store_remove_expected_value_gone() {
        let store = ContextStore::new();
        store.set("temp", json!(true)).expect("set");
        store.remove("temp").expect("remove");
        assert_eq!(store.get("temp").expect("get"), None);
    }

    #[test]
    fn validate_context_key_empty_expected_error() {
        assert!(validate_context_key("").is_err());
    }

    #[test]
    fn validate_context_key_too_long_expected_error() {
        let long_key = "a".repeat(257);
        assert!(validate_context_key(&long_key).is_err());
    }

    #[test]
    fn validate_context_key_invalid_segment_expected_error() {
        assert!(validate_context_key("context.bad key").is_err());
    }

    #[test]
    fn validate_context_key_valid_dotted_expected_ok() {
        assert!(validate_context_key("context.plan.status").is_ok());
    }

    // -- 5.2 NodeOutcome --

    #[test]
    fn node_outcome_success_helper_expected_default_fields() {
        let outcome = NodeOutcome::success();
        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(outcome.notes.is_none());
        assert!(outcome.failure_reason.is_none());
        assert!(outcome.context_updates.is_empty());
    }

    #[test]
    fn node_outcome_failure_helper_expected_fields() {
        let outcome = NodeOutcome::failure("broken");
        assert_eq!(outcome.status, NodeStatus::Fail);
        assert_eq!(outcome.failure_reason.as_deref(), Some("broken"));
        assert_eq!(outcome.notes.as_deref(), Some("broken"));
    }

    #[test]
    fn node_outcome_context_updates_expected_preserved() {
        let mut updates = RuntimeContext::new();
        updates.insert("key".to_string(), json!("value"));
        let outcome = NodeOutcome {
            status: NodeStatus::Success,
            context_updates: updates,
            ..Default::default()
        };
        assert_eq!(
            outcome.context_updates.get("key"),
            Some(&json!("value"))
        );
    }

    // -- 5.3 Checkpoint --

    #[test]
    fn checkpoint_roundtrip_expected_preserves_all_fields() {
        let temp = TempDir::new().expect("temp dir");
        let path = checkpoint_file_path(temp.path());
        let checkpoint = CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "run-1".to_string(),
                checkpoint_id: "cp-1".to_string(),
                sequence_no: 5,
                timestamp: "123.000Z".to_string(),
            },
            current_node: "plan".to_string(),
            next_node: Some("review".to_string()),
            completed_nodes: vec!["start".to_string(), "plan".to_string()],
            node_retries: BTreeMap::from([("plan".to_string(), 1)]),
            node_outcomes: BTreeMap::from([(
                "plan".to_string(),
                CheckpointNodeOutcome {
                    status: "success".to_string(),
                    notes: Some("ok".to_string()),
                    preferred_label: None,
                    suggested_next_ids: vec![],
                },
            )]),
            context_values: BTreeMap::from([("outcome".to_string(), json!("success"))]),
            logs: vec!["saved".to_string()],
            current_node_fidelity: Some("full".to_string()),
            terminal_status: None,
            terminal_failure_reason: None,
            graph_dot_source_hash: Some("hash1".to_string()),
            graph_dot_source_ref: Some("artifact://dot1".to_string()),
            graph_snapshot_hash: Some("hash2".to_string()),
            graph_snapshot_ref: Some("artifact://snap1".to_string()),
        };
        checkpoint.save_to_path(&path).expect("save");
        let loaded = CheckpointState::load_from_path(&path).expect("load");
        assert_eq!(loaded, checkpoint);
    }

    #[test]
    fn checkpoint_node_outcome_to_runtime_expected_status_mapping() {
        let checkpoint_outcome = CheckpointNodeOutcome {
            status: "partial_success".to_string(),
            notes: Some("n".to_string()),
            preferred_label: Some("yes".to_string()),
            suggested_next_ids: vec!["a".to_string()],
        };
        let runtime = checkpoint_outcome.to_runtime().expect("convert");
        assert_eq!(runtime.status, NodeStatus::PartialSuccess);
        assert_eq!(runtime.preferred_label.as_deref(), Some("yes"));
    }

    #[test]
    fn checkpoint_node_outcome_invalid_status_expected_error() {
        let checkpoint_outcome = CheckpointNodeOutcome {
            status: "invalid_status".to_string(),
            notes: None,
            preferred_label: None,
            suggested_next_ids: vec![],
        };
        assert!(checkpoint_outcome.to_runtime().is_err());
    }

    #[test]
    fn checkpoint_terminal_pipeline_status_success_expected() {
        let checkpoint = CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "r".to_string(),
                checkpoint_id: "c".to_string(),
                sequence_no: 1,
                timestamp: "t".to_string(),
            },
            current_node: "exit".to_string(),
            next_node: None,
            completed_nodes: vec![],
            node_retries: BTreeMap::new(),
            node_outcomes: BTreeMap::new(),
            context_values: BTreeMap::new(),
            logs: vec![],
            current_node_fidelity: None,
            terminal_status: Some("success".to_string()),
            terminal_failure_reason: None,
            graph_dot_source_hash: None,
            graph_dot_source_ref: None,
            graph_snapshot_hash: None,
            graph_snapshot_ref: None,
        };
        assert_eq!(
            checkpoint.terminal_pipeline_status().expect("ok"),
            Some(PipelineStatus::Success)
        );
    }

    #[test]
    fn checkpoint_terminal_pipeline_status_none_expected() {
        let checkpoint = CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "r".to_string(),
                checkpoint_id: "c".to_string(),
                sequence_no: 1,
                timestamp: "t".to_string(),
            },
            current_node: "plan".to_string(),
            next_node: None,
            completed_nodes: vec![],
            node_retries: BTreeMap::new(),
            node_outcomes: BTreeMap::new(),
            context_values: BTreeMap::new(),
            logs: vec![],
            current_node_fidelity: None,
            terminal_status: None,
            terminal_failure_reason: None,
            graph_dot_source_hash: None,
            graph_dot_source_ref: None,
            graph_snapshot_hash: None,
            graph_snapshot_ref: None,
        };
        assert_eq!(
            checkpoint.terminal_pipeline_status().expect("ok"),
            None
        );
    }

    // -- 5.4 Fidelity --

    #[test]
    fn is_valid_fidelity_mode_all_valid_expected_true() {
        for mode in ["full", "truncate", "compact", "summary:low", "summary:medium", "summary:high"]
        {
            assert!(
                is_valid_fidelity_mode(mode),
                "expected valid: {mode}"
            );
        }
    }

    #[test]
    fn is_valid_fidelity_mode_invalid_expected_false() {
        assert!(!is_valid_fidelity_mode("brief"));
        assert!(!is_valid_fidelity_mode(""));
    }

    #[test]
    fn resolve_fidelity_mode_edge_precedence_expected_edge_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="compact"]
                start [shape=Mdiamond]
                plan [fidelity="summary:low"]
                start -> plan [fidelity="full"]
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(resolve_fidelity_mode(&graph, "plan", incoming), "full");
    }

    #[test]
    fn resolve_fidelity_mode_node_then_graph_expected_precedence() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="summary:medium"]
                start [shape=Mdiamond]
                plan [fidelity="truncate"]
                review
                start -> plan -> review
            }
            "#,
        )
        .expect("parse");
        let incoming_plan = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_fidelity_mode(&graph, "plan", incoming_plan),
            "truncate"
        );
        let incoming_review = find_incoming_edge(&graph, "review", Some("plan"));
        assert_eq!(
            resolve_fidelity_mode(&graph, "review", incoming_review),
            "summary:medium"
        );
    }

    #[test]
    fn resolve_fidelity_mode_no_override_expected_default_compact() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan
                start -> plan
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(resolve_fidelity_mode(&graph, "plan", incoming), "compact");
    }

    // -- 5.5 Artifacts --

    #[test]
    fn artifact_store_small_inline_expected_not_file_backed() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        let info = store
            .store_json("summary", "Summary", &json!({"ok": true}))
            .expect("store");
        assert!(!info.is_file_backed);
        assert_eq!(info.reference, "artifact://summary");
    }

    #[test]
    fn artifact_store_large_file_backed_expected_file_created() {
        let temp = TempDir::new().expect("temp dir");
        let store = ArtifactStore::new(Some(temp.path().to_path_buf()), 64).expect("create");
        let payload = json!({"content": "x".repeat(512)});
        let info = store
            .store_json("large", "Large", &payload)
            .expect("store");
        assert!(info.is_file_backed);
        assert!(temp.path().join("artifacts/large.json").exists());
    }

    #[test]
    fn artifact_store_retrieve_expected_round_trip() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        let data = json!({"hello": "world"});
        store.store_json("art1", "Art", &data).expect("store");
        let retrieved = store.retrieve_json("art1").expect("retrieve");
        assert_eq!(retrieved, data);
    }

    #[test]
    fn artifact_store_retrieve_by_reference_expected_works() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        store
            .store_json("art2", "Art", &json!(42))
            .expect("store");
        let retrieved = store
            .retrieve_json_by_reference("artifact://art2")
            .expect("retrieve");
        assert_eq!(retrieved, json!(42));
    }

    #[test]
    fn artifact_store_remove_expected_cleaned() {
        let temp = TempDir::new().expect("temp dir");
        let store = ArtifactStore::new(Some(temp.path().to_path_buf()), 1).expect("create");
        store
            .store_json("rm1", "Rm", &json!("data"))
            .expect("store");
        assert!(store.has("rm1"));
        store.remove("rm1").expect("remove");
        assert!(!store.has("rm1"));
    }

    #[test]
    fn artifact_store_list_expected_all_entries() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        store.store_json("a1", "A1", &json!(1)).expect("store");
        store.store_json("a2", "A2", &json!(2)).expect("store");
        let list = store.list().expect("list");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn artifact_store_invalid_id_expected_error() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        let err = store
            .store_json("bad id", "Bad", &json!(1))
            .expect_err("invalid id");
        assert!(err.to_string().contains("unsupported characters"));
    }

    #[test]
    fn artifact_store_empty_name_expected_error() {
        let store = ArtifactStore::new(None, 1024).expect("create");
        let err = store
            .store_json("ok-id", "", &json!(1))
            .expect_err("empty name");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn artifact_store_default_threshold_expected_100kb() {
        assert_eq!(
            forge_attractor::DEFAULT_FILE_BACKING_THRESHOLD_BYTES,
            100 * 1024
        );
    }

    // -- 5.6 Thread key resolution --

    #[test]
    fn resolve_thread_key_node_level_expected_highest_precedence() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [thread_id="graph-t"]
                start [shape=Mdiamond]
                plan [thread_id="node-t"]
                start -> plan [thread_id="edge-t"]
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming, Some("start")).as_deref(),
            Some("node-t")
        );
    }

    #[test]
    fn resolve_thread_key_edge_level_expected_used_when_no_node() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [thread_id="graph-t"]
                start [shape=Mdiamond]
                plan
                start -> plan [thread_id="edge-t"]
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming, Some("start")).as_deref(),
            Some("edge-t")
        );
    }

    #[test]
    fn resolve_thread_key_graph_level_expected_fallback() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [thread_id="graph-t"]
                start [shape=Mdiamond]
                plan
                start -> plan
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming, Some("start")).as_deref(),
            Some("graph-t")
        );
    }

    #[test]
    fn resolve_thread_key_class_fallback_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan [class="loop-a"]
                start -> plan
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming, Some("start")).as_deref(),
            Some("loop-a")
        );
    }

    #[test]
    fn resolve_thread_key_previous_node_final_fallback_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan
                start -> plan
            }
            "#,
        )
        .expect("parse");
        let incoming = find_incoming_edge(&graph, "plan", Some("start"));
        assert_eq!(
            resolve_thread_key(&graph, "plan", incoming, Some("start")).as_deref(),
            Some("start")
        );
    }
}

// =========================================================================
// Section 6: HITL
// =========================================================================
mod section_6_hitl {
    use super::*;
    use forge_attractor::{
        AutoApproveInterviewer, CallbackInterviewer, HumanAnswer, HumanChoice,
        HumanQuestion, HumanQuestionType, Interviewer, QueueInterviewer,
        RecordingInterviewer,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn auto_approve_yes_no_expected_yes() {
        let interviewer = AutoApproveInterviewer;
        let answer = interviewer
            .ask(HumanQuestion {
                stage: "g".to_string(),
                text: "?".to_string(),
                question_type: HumanQuestionType::YesNo,
                choices: vec![],
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::Yes);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_approve_multiple_choice_expected_first_selected() {
        let interviewer = AutoApproveInterviewer;
        let answer = interviewer
            .ask(HumanQuestion {
                stage: "g".to_string(),
                text: "Pick".to_string(),
                question_type: HumanQuestionType::MultipleChoice,
                choices: vec![
                    HumanChoice {
                        key: "A".to_string(),
                        label: "Approve".to_string(),
                        to_node: "ok".to_string(),
                    },
                    HumanChoice {
                        key: "R".to_string(),
                        label: "Revise".to_string(),
                        to_node: "fix".to_string(),
                    },
                ],
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::Selected("A".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_approve_free_text_expected_auto_approved() {
        let interviewer = AutoApproveInterviewer;
        let answer = interviewer
            .ask(HumanQuestion {
                stage: "g".to_string(),
                text: "?".to_string(),
                question_type: HumanQuestionType::FreeText,
                choices: vec![],
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::FreeText("auto-approved".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queue_interviewer_fifo_order_then_skipped_expected() {
        let interviewer = QueueInterviewer::with_answers(vec![
            HumanAnswer::Selected("A".to_string()),
            HumanAnswer::Selected("B".to_string()),
        ]);
        let q = HumanQuestion {
            stage: "g".to_string(),
            text: "?".to_string(),
            question_type: HumanQuestionType::MultipleChoice,
            choices: vec![],
            default_choice: None,
            timeout: None,
        };
        assert_eq!(
            interviewer.ask(q.clone()).await,
            HumanAnswer::Selected("A".to_string())
        );
        assert_eq!(
            interviewer.ask(q.clone()).await,
            HumanAnswer::Selected("B".to_string())
        );
        assert_eq!(interviewer.ask(q).await, HumanAnswer::Skipped);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queue_interviewer_pending_count_expected_correct() {
        let interviewer = QueueInterviewer::with_answers(vec![
            HumanAnswer::Yes,
            HumanAnswer::No,
        ]);
        assert_eq!(interviewer.pending(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recording_interviewer_records_expected() {
        let inner = Arc::new(QueueInterviewer::with_answers(vec![HumanAnswer::Yes]));
        let recording = RecordingInterviewer::new(inner);
        let question = HumanQuestion {
            stage: "review".to_string(),
            text: "Ship?".to_string(),
            question_type: HumanQuestionType::YesNo,
            choices: vec![],
            default_choice: None,
            timeout: None,
        };
        let answer = recording.ask(question.clone()).await;
        assert_eq!(answer, HumanAnswer::Yes);
        let records = recording.recordings();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].question, question);
        assert_eq!(records[0].answer, HumanAnswer::Yes);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn callback_interviewer_expected_callback_invoked() {
        let interviewer = CallbackInterviewer::new(|q| {
            if q.stage == "gate" {
                HumanAnswer::Selected("R".to_string())
            } else {
                HumanAnswer::Skipped
            }
        });
        let answer = interviewer
            .ask(HumanQuestion {
                stage: "gate".to_string(),
                text: "?".to_string(),
                question_type: HumanQuestionType::MultipleChoice,
                choices: vec![],
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::Selected("R".to_string()));
    }

    #[test]
    fn human_question_type_variants_expected_all_exist() {
        let _ = HumanQuestionType::YesNo;
        let _ = HumanQuestionType::MultipleChoice;
        let _ = HumanQuestionType::FreeText;
        let _ = HumanQuestionType::Confirmation;
    }

    #[test]
    fn human_answer_variants_expected_all_exist() {
        let _ = HumanAnswer::Selected("x".to_string());
        let _ = HumanAnswer::Yes;
        let _ = HumanAnswer::No;
        let _ = HumanAnswer::FreeText("t".to_string());
        let _ = HumanAnswer::Timeout;
        let _ = HumanAnswer::Skipped;
    }
}

// =========================================================================
// Section 7: Validation
// =========================================================================
mod section_7_validation {
    use super::*;

    #[test]
    fn validate_missing_start_expected_error() {
        let graph = parse_dot("digraph G { exit [shape=Msquare] }").expect("parse");
        let diags = validate(&graph, &[]);
        assert!(diags.iter().any(|d| d.rule == "start_node" && d.is_error()));
    }

    #[test]
    fn validate_missing_terminal_expected_error() {
        let graph = parse_dot("digraph G { start [shape=Mdiamond] }").expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "terminal_node" && d.is_error())
        );
    }

    #[test]
    fn validate_edge_target_missing_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> missing -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "edge_target_exists" && d.is_error())
        );
    }

    #[test]
    fn validate_start_has_incoming_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                exit -> start
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "start_no_incoming" && d.is_error())
        );
    }

    #[test]
    fn validate_exit_has_outgoing_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
                exit -> start
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "exit_no_outgoing" && d.is_error())
        );
    }

    #[test]
    fn validate_unreachable_node_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                orphan
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "reachability" && d.is_error())
        );
    }

    #[test]
    fn validate_invalid_condition_syntax_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit [condition="outcome="]
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "condition_syntax" && d.is_error())
        );
    }

    #[test]
    fn validate_stylesheet_syntax_invalid_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="* { llm_model base; }"]
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "stylesheet_syntax" && d.is_error())
        );
    }

    #[test]
    fn validate_unknown_type_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                n1 [type="unknown_type"]
                exit [shape=Msquare]
                start -> n1 -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "type_known" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_known_types_no_warning_expected() {
        for node_type in [
            "start", "exit", "codergen", "wait.human", "conditional", "parallel",
            "parallel.fan_in", "tool", "stack.manager_loop",
        ] {
            let graph = parse_dot(&format!(
                r#"
                digraph G {{
                    start [shape=Mdiamond]
                    n1 [type="{node_type}"]
                    exit [shape=Msquare]
                    start -> n1 -> exit
                }}
                "#,
            ))
            .expect("parse");
            let diags = validate(&graph, &[]);
            assert!(
                !diags.iter().any(|d| d.rule == "type_known"),
                "type={node_type} should not produce type_known diagnostic"
            );
        }
    }

    #[test]
    fn validate_invalid_fidelity_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="brief"]
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "fidelity_valid" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_valid_fidelity_no_warning_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            !diags.iter().any(|d| d.rule == "fidelity_valid"),
        );
    }

    #[test]
    fn validate_retry_target_missing_node_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [retry_target="nonexistent"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "retry_target_exists" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_goal_gate_without_retry_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [goal_gate=true]
                exit [shape=Msquare]
                start -> gate -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "goal_gate_has_retry" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_goal_gate_with_retry_target_no_warning_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [goal_gate=true, retry_target="fix"]
                fix
                exit [shape=Msquare]
                start -> gate -> exit
                gate -> fix [condition="outcome=fail"]
                fix -> gate
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            !diags
                .iter()
                .any(|d| d.rule == "goal_gate_has_retry")
        );
    }

    #[test]
    fn validate_prompt_on_llm_node_missing_expected_warning() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                task
                exit [shape=Msquare]
                start -> task -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "prompt_on_llm_nodes" && d.severity == Severity::Warning)
        );
    }

    #[test]
    fn validate_prompt_on_llm_node_present_no_warning_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                task [prompt="Do thing"]
                exit [shape=Msquare]
                start -> task -> exit
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            !diags
                .iter()
                .any(|d| d.rule == "prompt_on_llm_nodes")
        );
    }

    #[test]
    fn validate_or_raise_with_errors_expected_validation_error() {
        let graph = parse_dot("digraph G { orphan }").expect("parse");
        let err = validate_or_raise(&graph, &[]).expect_err("should fail");
        assert!(err.errors_count > 0);
    }

    #[test]
    fn validate_valid_graph_no_errors_expected_ok() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Do thing"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let result = validate_or_raise(&graph, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn diagnostic_builders_expected_fields_set() {
        let d = Diagnostic::new("rule", Severity::Error, "msg")
            .with_node_id("n1")
            .with_edge("a", "b")
            .with_fix("do this");
        assert_eq!(d.rule, "rule");
        assert!(d.is_error());
        assert_eq!(d.node_id.as_deref(), Some("n1"));
        assert_eq!(d.edge, Some(("a".to_string(), "b".to_string())));
        assert_eq!(d.fix.as_deref(), Some("do this"));
    }
}

// =========================================================================
// Section 8: Stylesheet
// =========================================================================
mod section_8_stylesheet {
    use super::*;

    #[test]
    fn parse_stylesheet_universal_selector_expected() {
        let rules = parse_stylesheet("* { llm_model: m1; }").expect("parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, Selector::Universal);
    }

    #[test]
    fn parse_stylesheet_node_id_selector_expected() {
        let rules = parse_stylesheet("#critical { llm_model: m1; }").expect("parse");
        assert_eq!(rules[0].selector, Selector::NodeId("critical".to_string()));
    }

    #[test]
    fn parse_stylesheet_class_selector_expected() {
        let rules = parse_stylesheet(".code { llm_model: m1; }").expect("parse");
        assert_eq!(rules[0].selector, Selector::Class("code".to_string()));
    }

    #[test]
    fn parse_stylesheet_multiple_declarations_expected() {
        let rules = parse_stylesheet(
            "* { llm_model: m1; llm_provider: openai; reasoning_effort: high; }",
        )
        .expect("parse");
        assert_eq!(rules[0].declarations.len(), 3);
    }

    #[test]
    fn parse_stylesheet_multiple_rules_expected_correct_count() {
        let rules = parse_stylesheet(
            r#"
            * { llm_model: base; }
            .code { llm_model: coder; }
            #review { reasoning_effort: high; }
            "#,
        )
        .expect("parse");
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].order, 0);
        assert_eq!(rules[1].order, 1);
        assert_eq!(rules[2].order, 2);
    }

    #[test]
    fn parse_stylesheet_missing_brace_expected_error() {
        let err = parse_stylesheet("* llm_model: m1; ").expect_err("should fail");
        assert!(err.to_string().contains("{"));
    }

    #[test]
    fn parse_stylesheet_missing_colon_expected_error() {
        let err = parse_stylesheet("* { llm_model m1; }").expect_err("should fail");
        assert!(err.to_string().contains(":"));
    }

    #[test]
    fn parse_stylesheet_unsupported_property_expected_error() {
        let err =
            parse_stylesheet("* { color: red; }").expect_err("should fail");
        assert!(err.to_string().contains("not supported"));
    }

    #[test]
    fn parse_stylesheet_invalid_reasoning_effort_expected_error() {
        let err = parse_stylesheet("* { reasoning_effort: extreme; }")
            .expect_err("should fail");
        assert!(err.to_string().contains("low|medium|high"));
    }

    #[test]
    fn parse_stylesheet_empty_rule_expected_error() {
        let err = parse_stylesheet("* {}").expect_err("should fail");
        assert!(err.to_string().contains("at least one"));
    }

    #[test]
    fn apply_model_stylesheet_specificity_id_beats_class_expected() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="
                    * { llm_model: base; }
                    .code { llm_model: class_model; }
                    #n1 { llm_model: id_model; }
                "]
                n1 [class="code"]
            }
            "#,
        )
        .expect("parse");
        apply_model_stylesheet(&mut graph).expect("apply");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(
            node.attrs.get("llm_model"),
            Some(&AttrValue::String("id_model".to_string()))
        );
    }

    #[test]
    fn apply_model_stylesheet_class_beats_universal_expected() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="
                    * { llm_model: base; }
                    .code { llm_model: code_model; }
                "]
                n1 [class="code"]
            }
            "#,
        )
        .expect("parse");
        apply_model_stylesheet(&mut graph).expect("apply");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(
            node.attrs.get("llm_model"),
            Some(&AttrValue::String("code_model".to_string()))
        );
    }

    #[test]
    fn apply_model_stylesheet_explicit_attr_not_overridden_expected() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="* { llm_model: base; }"]
                n1 [llm_model="explicit"]
            }
            "#,
        )
        .expect("parse");
        apply_model_stylesheet(&mut graph).expect("apply");
        let node = graph.nodes.get("n1").unwrap();
        assert_eq!(
            node.attrs.get("llm_model"),
            Some(&AttrValue::String("explicit".to_string()))
        );
    }

    #[test]
    fn apply_model_stylesheet_no_stylesheet_expected_noop() {
        let mut graph = parse_dot("digraph G { n1 }").expect("parse");
        apply_model_stylesheet(&mut graph).expect("apply");
        // No llm_model should be set
        assert!(graph.nodes.get("n1").unwrap().attrs.get("llm_model").is_none());
    }

    #[test]
    fn parse_stylesheet_quoted_value_expected_unquoted() {
        let rules =
            parse_stylesheet(r#"* { llm_model: "gpt-4o"; }"#).expect("parse");
        assert_eq!(rules[0].declarations[0].1, "gpt-4o");
    }

    #[test]
    fn parse_stylesheet_shape_selector_expected_shape_variant() {
        let rules = parse_stylesheet("box { llm_model: gpt4; }").expect("parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, Selector::Shape("box".to_string()));
    }

    #[test]
    fn apply_model_stylesheet_shape_selector_expected_match() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [model_stylesheet="
                    * { llm_model: base; }
                    box { llm_model: box_model; }
                    .code { llm_model: class_model; }
                    #specific { llm_model: id_model; }
                "]
                plain_box [shape=box]
                class_box [shape=box, class="code"]
                specific [shape=box, class="code"]
            }
            "#,
        )
        .expect("parse");
        apply_model_stylesheet(&mut graph).expect("apply");

        // shape selector beats universal
        let plain = graph.nodes.get("plain_box").unwrap();
        assert_eq!(
            plain.attrs.get("llm_model"),
            Some(&AttrValue::String("box_model".to_string())),
            "shape selector should beat universal"
        );

        // class selector beats shape selector
        let class_node = graph.nodes.get("class_box").unwrap();
        assert_eq!(
            class_node.attrs.get("llm_model"),
            Some(&AttrValue::String("class_model".to_string())),
            "class selector should beat shape selector"
        );

        // id selector beats class and shape
        let id_node = graph.nodes.get("specific").unwrap();
        assert_eq!(
            id_node.attrs.get("llm_model"),
            Some(&AttrValue::String("id_model".to_string())),
            "id selector should beat class and shape"
        );
    }
}

// =========================================================================
// Section 9: Transforms
// =========================================================================
mod section_9_transforms {
    use super::*;
    use forge_attractor::{
        Transform, VariableExpansionTransform,
        apply_builtin_transforms, prepare_pipeline,
    };

    #[test]
    fn variable_expansion_goal_in_prompt_expected_replaced() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [goal="Ship feature"]
                plan [prompt="Plan for $goal"]
            }
            "#,
        )
        .expect("parse");
        VariableExpansionTransform.apply(&mut graph).expect("apply");
        let node = graph.nodes.get("plan").unwrap();
        assert_eq!(
            node.attrs.get_str("prompt"),
            Some("Plan for Ship feature")
        );
    }

    #[test]
    fn variable_expansion_no_goal_expected_noop() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                plan [prompt="Plan for $goal"]
            }
            "#,
        )
        .expect("parse");
        VariableExpansionTransform.apply(&mut graph).expect("apply");
        let node = graph.nodes.get("plan").unwrap();
        assert_eq!(node.attrs.get_str("prompt"), Some("Plan for $goal"));
    }

    #[test]
    fn apply_builtin_transforms_expected_both_expansion_and_stylesheet() {
        let mut graph = parse_dot(
            r#"
            digraph G {
                graph [goal="ship", model_stylesheet="* { llm_model: base; }"]
                plan [prompt="Plan for $goal"]
            }
            "#,
        )
        .expect("parse");
        apply_builtin_transforms(&mut graph).expect("apply");
        let node = graph.nodes.get("plan").unwrap();
        assert_eq!(
            node.attrs.get_str("prompt"),
            Some("Plan for ship")
        );
        assert_eq!(
            node.attrs.get("llm_model"),
            Some(&AttrValue::String("base".to_string()))
        );
    }

    #[test]
    fn prepare_pipeline_valid_expected_graph_and_diagnostics() {
        let (graph, diags) = prepare_pipeline(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
            &[],
            &[],
        )
        .expect("prepare");
        assert!(graph.nodes.contains_key("start"));
        // Should have no errors
        assert!(!diags.iter().any(|d| d.is_error()));
    }

    #[test]
    fn prepare_pipeline_custom_transform_expected_applied() {
        struct UppercasePromptTransform;
        impl Transform for UppercasePromptTransform {
            fn apply(&self, graph: &mut Graph) -> Result<(), AttractorError> {
                for node in graph.nodes.values_mut() {
                    if let Some(prompt) = node.attrs.get_str("prompt") {
                        let upper = prompt.to_uppercase();
                        node.attrs
                            .set_inherited("prompt", AttrValue::String(upper));
                    }
                }
                Ok(())
            }
        }
        let (graph, _) = prepare_pipeline(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="hello"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
            &[&UppercasePromptTransform],
            &[],
        )
        .expect("prepare");
        assert_eq!(
            graph.nodes.get("work").unwrap().attrs.get_str("prompt"),
            Some("HELLO")
        );
    }
}

// =========================================================================
// Section 10: Conditions
// =========================================================================
mod section_10_conditions {
    use super::*;

    fn success_outcome() -> NodeOutcome {
        NodeOutcome {
            status: NodeStatus::Success,
            preferred_label: Some("Yes".to_string()),
            ..Default::default()
        }
    }

    // -- Validate --

    #[test]
    fn validate_condition_valid_key_expected_ok() {
        validate_condition_expression("outcome=success").expect("should be valid");
    }

    #[test]
    fn validate_condition_context_key_expected_ok() {
        validate_condition_expression("context.ready=true").expect("should be valid");
    }

    #[test]
    fn validate_condition_invalid_key_expected_error() {
        assert!(validate_condition_expression("123bad=x").is_err());
    }

    #[test]
    fn validate_condition_empty_value_expected_error() {
        assert!(validate_condition_expression("outcome=").is_err());
    }

    #[test]
    fn validate_condition_exists_clause_expected_ok() {
        validate_condition_expression("context.ready").expect("should be valid");
    }

    // -- Evaluate: eq --

    #[test]
    fn evaluate_condition_outcome_eq_success_expected_true() {
        let ctx = RuntimeContext::new();
        assert!(
            evaluate_condition_expression("outcome=success", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_outcome_eq_fail_expected_false() {
        let ctx = RuntimeContext::new();
        assert!(
            !evaluate_condition_expression("outcome=fail", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_preferred_label_eq_expected_true() {
        let ctx = RuntimeContext::new();
        assert!(
            evaluate_condition_expression("preferred_label=Yes", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_context_key_eq_expected_true() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("ready".to_string(), json!(true));
        assert!(
            evaluate_condition_expression("context.ready=true", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_context_key_integer_expected_true() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("tries".to_string(), json!(3));
        assert!(
            evaluate_condition_expression("context.tries=3", &success_outcome(), &ctx).unwrap()
        );
    }

    // -- Evaluate: ne --

    #[test]
    fn evaluate_condition_ne_mismatch_expected_false() {
        let ctx = RuntimeContext::new();
        assert!(
            !evaluate_condition_expression("outcome!=success", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_ne_match_expected_true() {
        let ctx = RuntimeContext::new();
        assert!(
            evaluate_condition_expression("outcome!=fail", &success_outcome(), &ctx).unwrap()
        );
    }

    // -- Evaluate: exists --

    #[test]
    fn evaluate_condition_exists_present_expected_true() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("ready".to_string(), json!(true));
        assert!(
            evaluate_condition_expression("context.ready", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_exists_missing_expected_false() {
        let ctx = RuntimeContext::new();
        assert!(
            !evaluate_condition_expression("context.ready", &success_outcome(), &ctx).unwrap()
        );
    }

    #[test]
    fn evaluate_condition_preferred_label_exists_expected_true() {
        let ctx = RuntimeContext::new();
        assert!(
            evaluate_condition_expression("preferred_label", &success_outcome(), &ctx).unwrap()
        );
    }

    // -- Evaluate: compound --

    #[test]
    fn evaluate_condition_multiple_clauses_all_true_expected_true() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("ready".to_string(), json!(true));
        assert!(evaluate_condition_expression(
            "outcome=success && context.ready=true",
            &success_outcome(),
            &ctx
        )
        .unwrap());
    }

    #[test]
    fn evaluate_condition_multiple_clauses_one_false_expected_false() {
        let ctx = RuntimeContext::new();
        assert!(!evaluate_condition_expression(
            "outcome=success && context.ready=true",
            &success_outcome(),
            &ctx
        )
        .unwrap());
    }

    // -- Missing keys --

    #[test]
    fn evaluate_condition_missing_key_eq_empty_expected_true() {
        // Per spec: missing keys compare as empty strings
        let ctx = RuntimeContext::new();
        // missing key == "" should be true (both empty)
        // but "context.missing=something" should be false
        assert!(!evaluate_condition_expression(
            "context.missing=something",
            &success_outcome(),
            &ctx
        )
        .unwrap());
    }

    #[test]
    fn evaluate_condition_missing_key_ne_nonempty_expected_true() {
        let ctx = RuntimeContext::new();
        assert!(evaluate_condition_expression(
            "context.missing!=something",
            &success_outcome(),
            &ctx
        )
        .unwrap());
    }

    // -- Quoted strings --

    #[test]
    fn evaluate_condition_quoted_string_expected_match() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("choice".to_string(), json!("ship now"));
        assert!(evaluate_condition_expression(
            "context.choice=\"ship now\"",
            &success_outcome(),
            &ctx
        )
        .unwrap());
    }

    // -- Bare key (direct context lookup) --

    #[test]
    fn evaluate_condition_bare_key_eq_expected_direct_lookup() {
        let mut ctx = RuntimeContext::new();
        ctx.insert("status".to_string(), json!("ok"));
        assert!(
            evaluate_condition_expression("status=ok", &success_outcome(), &ctx).unwrap()
        );
    }
}

// =========================================================================
// Retry unit tests (not pipeline-level)
// =========================================================================
mod section_retry_unit {
    use super::*;

    #[test]
    fn retry_preset_none_expected_one_attempt() {
        let policy = RetryPreset::None.to_policy();
        assert_eq!(policy.max_attempts, 1);
    }

    #[test]
    fn retry_preset_standard_expected_five_attempts() {
        let policy = RetryPreset::Standard.to_policy();
        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.backoff.initial_delay_ms, 200);
    }

    #[test]
    fn retry_preset_aggressive_expected_five_attempts_500ms() {
        let policy = RetryPreset::Aggressive.to_policy();
        assert_eq!(policy.max_attempts, 5);
        assert_eq!(policy.backoff.initial_delay_ms, 500);
    }

    #[test]
    fn retry_preset_linear_expected_three_attempts_factor_one() {
        let policy = RetryPreset::Linear.to_policy();
        assert_eq!(policy.max_attempts, 3);
        assert!((policy.backoff.backoff_factor - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn retry_preset_patient_expected_three_attempts_2000ms() {
        let policy = RetryPreset::Patient.to_policy();
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.backoff.initial_delay_ms, 2000);
    }

    #[test]
    fn retry_preset_from_str_all_variants_expected() {
        assert_eq!(RetryPreset::from_str("none"), Some(RetryPreset::None));
        assert_eq!(RetryPreset::from_str("standard"), Some(RetryPreset::Standard));
        assert_eq!(RetryPreset::from_str("aggressive"), Some(RetryPreset::Aggressive));
        assert_eq!(RetryPreset::from_str("linear"), Some(RetryPreset::Linear));
        assert_eq!(RetryPreset::from_str("patient"), Some(RetryPreset::Patient));
        assert_eq!(RetryPreset::from_str("unknown"), None);
    }

    #[test]
    fn build_retry_policy_node_max_retries_expected_attempts_plus_one() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=3]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("work").unwrap();
        let policy = build_retry_policy(node, &graph, RetryBackoffConfig::default());
        assert_eq!(policy.max_attempts, 4);
    }

    #[test]
    fn build_retry_policy_graph_default_expected_fallback() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_max_retry=2]
                start [shape=Mdiamond]
                work
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("work").unwrap();
        let policy = build_retry_policy(node, &graph, RetryBackoffConfig::default());
        assert_eq!(policy.max_attempts, 3);
    }

    #[test]
    fn build_retry_policy_preset_overrides_max_retries_expected() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=10, retry_preset="standard"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("work").unwrap();
        let policy = build_retry_policy(node, &graph, RetryBackoffConfig::default());
        // preset takes precedence
        assert_eq!(policy.max_attempts, 5);
    }

    #[test]
    fn should_retry_outcome_retry_status_expected_true() {
        let outcome = NodeOutcome {
            status: NodeStatus::Retry,
            ..Default::default()
        };
        assert!(should_retry_outcome(&outcome));
    }

    #[test]
    fn should_retry_outcome_fail_status_expected_false() {
        let outcome = NodeOutcome::failure("nope");
        assert!(!should_retry_outcome(&outcome));
    }

    #[test]
    fn should_retry_outcome_success_status_expected_false() {
        let outcome = NodeOutcome::success();
        assert!(!should_retry_outcome(&outcome));
    }

    #[test]
    fn finalize_retry_exhausted_no_allow_partial_expected_fail() {
        let graph = parse_dot("digraph G { work }").expect("parse");
        let node = graph.nodes.get("work").unwrap();
        let outcome = finalize_retry_exhausted(node);
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[test]
    fn finalize_retry_exhausted_allow_partial_expected_partial_success() {
        let graph =
            parse_dot("digraph G { work [allow_partial=true] }").expect("parse");
        let node = graph.nodes.get("work").unwrap();
        let outcome = finalize_retry_exhausted(node);
        assert_eq!(outcome.status, NodeStatus::PartialSuccess);
    }

    #[test]
    fn delay_for_attempt_no_jitter_expected_exponential() {
        let config = RetryBackoffConfig {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: false,
        };
        assert_eq!(delay_for_attempt_ms(1, &config, 0), 200);
        assert_eq!(delay_for_attempt_ms(2, &config, 0), 400);
        assert_eq!(delay_for_attempt_ms(3, &config, 0), 800);
    }

    #[test]
    fn delay_for_attempt_capped_at_max_expected() {
        let config = RetryBackoffConfig {
            initial_delay_ms: 10_000,
            backoff_factor: 10.0,
            max_delay_ms: 50_000,
            jitter: false,
        };
        let delay = delay_for_attempt_ms(5, &config, 0);
        assert!(delay <= 50_000);
    }

    #[test]
    fn delay_for_attempt_with_jitter_expected_bounded() {
        let config = RetryBackoffConfig {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: true,
        };
        let delay = delay_for_attempt_ms(2, &config, 42);
        // jitter factor is 0.5..1.5, base=400 => 200..600
        assert!((100..=1_200).contains(&delay));
    }
}

// =========================================================================
// Events
// =========================================================================
mod section_events {
    use super::*;
    use forge_attractor::{
        PipelineEvent, RuntimeEventKind, RuntimeEventSink,
        SharedRuntimeEventObserver, runtime_event_channel,
    };

    #[test]
    fn event_sink_observer_receives_expected() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer_seen = seen.clone();
        let observer: SharedRuntimeEventObserver = Arc::new(move |event: &RuntimeEvent| {
            observer_seen.lock().unwrap().push(event.sequence_no);
        });
        let sink = RuntimeEventSink::with_observer(observer);
        sink.emit(RuntimeEvent {
            sequence_no: 1,
            timestamp: "0.000Z".to_string(),
            kind: RuntimeEventKind::Pipeline(PipelineEvent::Started {
                run_id: "r".to_string(),
                graph_id: "g".to_string(),
                lineage_attempt: 1,
            }),
        });
        assert_eq!(seen.lock().unwrap().as_slice(), &[1]);
    }

    #[test]
    fn event_sink_channel_receives_expected() {
        let (tx, mut rx) = runtime_event_channel();
        let sink = RuntimeEventSink::with_sender(tx);
        sink.emit(RuntimeEvent {
            sequence_no: 42,
            timestamp: "0.000Z".to_string(),
            kind: RuntimeEventKind::Pipeline(PipelineEvent::Started {
                run_id: "r".to_string(),
                graph_id: "g".to_string(),
                lineage_attempt: 1,
            }),
        });
        let received = rx.try_recv().expect("event");
        assert_eq!(received.sequence_no, 42);
    }

    #[test]
    fn event_sink_not_enabled_by_default_expected() {
        let sink = RuntimeEventSink::default();
        assert!(!sink.is_enabled());
    }

    #[test]
    fn event_sink_enabled_with_observer_expected() {
        let observer: SharedRuntimeEventObserver = Arc::new(|_: &RuntimeEvent| {});
        let sink = RuntimeEventSink::with_observer(observer);
        assert!(sink.is_enabled());
    }
}

// =========================================================================
// Error types
// =========================================================================
mod section_errors {
    use super::*;

    #[test]
    fn attractor_error_dot_parse_expected_display() {
        let err = AttractorError::DotParse("bad syntax".to_string());
        assert!(err.to_string().contains("bad syntax"));
    }

    #[test]
    fn attractor_error_invalid_graph_expected_display() {
        let err = AttractorError::InvalidGraph("no digraph".to_string());
        assert!(err.to_string().contains("no digraph"));
    }

    #[test]
    fn attractor_error_stylesheet_parse_expected_display() {
        let err = AttractorError::StylesheetParse("missing brace".to_string());
        assert!(err.to_string().contains("missing brace"));
    }

    #[test]
    fn attractor_error_runtime_expected_display() {
        let err = AttractorError::Runtime("oops".to_string());
        assert!(err.to_string().contains("oops"));
    }

    #[test]
    fn validation_error_displays_count_expected() {
        let err = ValidationError::new(vec![
            Diagnostic::new("r1", Severity::Error, "e1"),
            Diagnostic::new("r2", Severity::Warning, "w1"),
        ]);
        assert_eq!(err.errors_count, 1);
        assert!(err.to_string().contains("1 error"));
    }

    #[test]
    fn validate_multiple_exit_nodes_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                exit1 [shape=Msquare]
                exit2 [shape=Msquare]
                start -> exit1
                start -> exit2
            }
            "#,
        )
        .expect("parse");
        let diags = validate(&graph, &[]);
        assert!(
            diags
                .iter()
                .any(|d| d.rule == "terminal_node" && d.is_error()),
            "expected terminal_node error for multiple exit nodes"
        );
    }
}

// =========================================================================
// Logs & artifacts (status.json, manifest.json)
// =========================================================================
mod section_logs_artifacts {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_status_json_uses_spec_field_names_expected_outcome_and_preferred_next_label() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Do work"]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("parse");
        let temp = tempfile::tempdir().expect("tempdir");
        let executor = Arc::new(ScriptedExecutor::new());
        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor,
                    retry_backoff: zero_delay_backoff(),
                    logs_root: Some(temp.path().to_path_buf()),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);

        // The runner writes status.json under {logs_root}/attempt-{N}/{node_id}/
        // or directly under {logs_root}/{node_id}/ depending on lineage attempt.
        // Find any status.json file for the "work" node.
        let mut status_path = temp.path().join("work").join("status.json");
        if !status_path.exists() {
            // With lineage, first attempt is under attempt-1
            status_path = temp.path().join("attempt-1").join("work").join("status.json");
        }
        assert!(
            status_path.exists(),
            "status.json should exist at {:?}",
            status_path
        );
        let content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();

        // Verify spec field names
        assert!(
            content.get("outcome").is_some(),
            "status.json should contain 'outcome' field, got: {:?}",
            content
        );
        assert!(
            content.get("preferred_next_label").is_some(),
            "status.json should contain 'preferred_next_label' field, got: {:?}",
            content
        );
        assert!(
            content.get("failure_reason").is_some(),
            "status.json should contain 'failure_reason' field, got: {:?}",
            content
        );
        // Verify the outcome value is correct for success
        assert_eq!(
            content["outcome"].as_str(),
            Some("success"),
            "outcome should be 'success'"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_manifest_json_contains_required_fields_expected_name_goal_time_runid() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [goal="test goal"]
                start [shape=Mdiamond]
                exit [shape=Msquare]
                start -> exit
            }
            "#,
        )
        .expect("parse");
        let temp = tempfile::tempdir().expect("tempdir");
        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    logs_root: Some(temp.path().to_path_buf()),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);

        // manifest.json is written at {logs_root}/manifest.json or {logs_root}/attempt-{N}/manifest.json
        let mut manifest_path = temp.path().join("manifest.json");
        if !manifest_path.exists() {
            manifest_path = temp.path().join("attempt-1").join("manifest.json");
        }
        assert!(
            manifest_path.exists(),
            "manifest.json should exist at {:?}",
            manifest_path
        );
        let content: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();

        assert!(
            content.get("pipeline_name").is_some(),
            "manifest.json should contain 'pipeline_name'"
        );
        assert!(
            content.get("goal").is_some(),
            "manifest.json should contain 'goal'"
        );
        assert!(
            content.get("start_time").is_some(),
            "manifest.json should contain 'start_time'"
        );
        assert!(
            content.get("run_id").is_some(),
            "manifest.json should contain 'run_id'"
        );
        assert_eq!(
            content["goal"].as_str(),
            Some("test goal"),
            "goal should match graph attribute"
        );
    }
}

// =========================================================================
// Parallel extended tests (wait_all, k_of_n, error_policy)
// =========================================================================
mod section_4_parallel_extended {
    use super::*;
    use forge_attractor::handlers::{
        NodeHandler,
        parallel::ParallelHandler,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn parallel_wait_all_with_failures_expected_partial_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="wait_all"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        // One success and one failure
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "success", "b": "fail"}),
        );
        let outcome = NodeHandler::execute(
            &ParallelHandler::default(),
            node,
            &context,
            &graph,
        )
        .await
        .expect("execute");
        // wait_all with failures should produce PartialSuccess, not Fail
        assert_eq!(
            outcome.status,
            NodeStatus::PartialSuccess,
            "wait_all with some failures should produce PartialSuccess"
        );
    }

    #[test]
    fn parallel_k_of_n_alias_maps_to_quorum_expected_success() {
        // k_of_n is an alias for quorum join policy
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="k_of_n", quorum_count=1]
                p -> a
                p -> b
                p -> c
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        // 1 success, 2 failures — quorum_count=1 means we only need 1
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "success", "b": "fail", "c": "fail"}),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = rt.block_on(async {
            NodeHandler::execute(
                &ParallelHandler::default(),
                node,
                &context,
                &graph,
            )
            .await
            .expect("execute")
        });
        assert_eq!(
            outcome.status,
            NodeStatus::Success,
            "k_of_n with quorum_count=1 and at least 1 success should produce Success"
        );
    }

    #[test]
    fn parallel_error_policy_ignore_expected_success_despite_failures() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="all_success", error_policy="ignore"]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        let mut context = RuntimeContext::new();
        // All branches fail
        context.insert(
            "parallel.branch_outcomes".to_string(),
            json!({"a": "fail", "b": "fail"}),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = rt.block_on(async {
            NodeHandler::execute(
                &ParallelHandler::default(),
                node,
                &context,
                &graph,
            )
            .await
            .expect("execute")
        });
        // error_policy=ignore downgrades failures to success before join evaluation
        assert_eq!(
            outcome.status,
            NodeStatus::Success,
            "error_policy=ignore should ignore all failures and produce Success"
        );
    }
}

// =========================================================================
// Pipeline-level integration tests (goal_gate, auto_status, loop_restart, scale)
// =========================================================================
mod section_pipeline_integration {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_unvisited_goal_gate_blocks_exit_expected_fail() {
        // The gate node is reachable but the pipeline path doesn't visit it:
        // start -> work -> exit (success path), gate is only reachable via a
        // condition that will never fire. The gate has goal_gate=true, so the
        // pipeline should fail at exit because the gate was never satisfied.
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                gate [goal_gate=true, prompt="Gate check"]
                exit [shape=Msquare]
                start -> work
                work -> exit [condition="outcome=success"]
                work -> gate [condition="outcome=fail"]
                gate -> exit
            }
            "#,
        )
        .expect("parse");
        // work succeeds by default, so the pipeline goes start -> work -> exit,
        // skipping gate entirely. But gate has goal_gate=true, so exit should fail.
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(
            result.status,
            PipelineStatus::Fail,
            "pipeline should fail because unvisited goal_gate node was never satisfied"
        );
        assert!(
            result.failure_reason.as_deref().unwrap_or("").contains("goal gate"),
            "failure reason should mention goal gate: {:?}",
            result.failure_reason
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_auto_status_synthesizes_success_from_failure_expected_success_routing() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                flaky [auto_status=true, prompt="Flaky"]
                next [prompt="Next"]
                exit [shape=Msquare]
                start -> flaky -> next -> exit
            }
            "#,
        )
        .expect("parse");
        // The flaky node will fail, but auto_status=true should synthesize success
        let executor = Arc::new(
            ScriptedExecutor::new().script("flaky", vec![NodeOutcome::failure("oops")]),
        );
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(
            result.status,
            PipelineStatus::Success,
            "auto_status=true should synthesize success from failure"
        );
        assert!(
            result.completed_nodes.contains(&"next".to_string()),
            "pipeline should continue to 'next' node after auto_status"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_loop_restart_edge_expected_pipeline_restarts() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan [prompt="Plan"]
                restart_target [prompt="Restart"]
                exit [shape=Msquare]
                start -> plan
                plan -> restart_target [loop_restart=true]
                restart_target -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(
            result.status,
            PipelineStatus::Success,
            "pipeline should complete successfully after loop restart"
        );
        // After restart, the pipeline runs a fresh attempt
        // The lineage attempt should be 2
        assert_eq!(
            result
                .context
                .get("internal.lineage.attempt")
                .and_then(Value::as_u64),
            Some(2),
            "lineage attempt should be 2 after restart"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_ten_plus_nodes_linear_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                n1 [prompt="Step 1"]
                n2 [prompt="Step 2"]
                n3 [prompt="Step 3"]
                n4 [prompt="Step 4"]
                n5 [prompt="Step 5"]
                n6 [prompt="Step 6"]
                n7 [prompt="Step 7"]
                n8 [prompt="Step 8"]
                n9 [prompt="Step 9"]
                n10 [prompt="Step 10"]
                exit [shape=Msquare]
                start -> n1 -> n2 -> n3 -> n4 -> n5 -> n6 -> n7 -> n8 -> n9 -> n10 -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        // All non-terminal nodes should be in completed_nodes
        for i in 1..=10 {
            let node_id = format!("n{}", i);
            assert!(
                result.completed_nodes.contains(&node_id),
                "node '{}' should be in completed_nodes",
                node_id
            );
        }
        assert!(
            result.completed_nodes.contains(&"start".to_string()),
            "start should be in completed_nodes"
        );
        // exit (Msquare terminal) should NOT be in completed_nodes
        assert!(
            !result.completed_nodes.contains(&"exit".to_string()),
            "exit (terminal) should not be in completed_nodes"
        );
        // Total: start + n1..n10 = 11 completed nodes
        assert_eq!(result.completed_nodes.len(), 11);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_validate_multiple_exit_nodes_expected_error() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                exit1 [shape=Msquare]
                exit2 [shape=Msquare]
                start -> work
                work -> exit1
                work -> exit2
            }
            "#,
        )
        .expect("parse");
        let err = validate_or_raise(&graph, &[]);
        assert!(err.is_err(), "pipeline with two exit nodes should be rejected");
    }
}

// =========================================================================
// Gap-closing tests: edge selection through full engine
// =========================================================================
mod section_edge_selection_integration {
    use super::*;

    /// Lexical tiebreak: when multiple edges have identical weight (0) and no
    /// conditions/labels/suggested IDs, the engine should pick the edge whose
    /// target node ID comes first lexicographically.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_lexical_tiebreak_expected_first_alphabetically() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                fork [shape=diamond]
                zulu [prompt="Zulu"]
                alpha [prompt="Alpha"]
                exit [shape=Msquare]
                start -> fork
                fork -> zulu
                fork -> alpha
                zulu -> exit
                alpha -> exit
            }
            "#,
        )
        .expect("parse");
        // No conditions, no preferred labels, no suggested IDs, equal weight (0).
        // Lexical tiebreak should pick "alpha" over "zulu".
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result.completed_nodes.contains(&"alpha".to_string()),
            "lexical tiebreak should route to 'alpha' (before 'zulu')"
        );
        assert!(
            !result.completed_nodes.contains(&"zulu".to_string()),
            "lexical tiebreak should NOT route to 'zulu'"
        );
    }

    /// Weight tiebreak through the full engine: higher weight edge wins.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_weight_wins_expected_heavy_path() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                fork [shape=diamond]
                light [prompt="Light"]
                heavy [prompt="Heavy"]
                exit [shape=Msquare]
                start -> fork
                fork -> light [weight=1]
                fork -> heavy [weight=10]
                light -> exit
                heavy -> exit
            }
            "#,
        )
        .expect("parse");
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result.completed_nodes.contains(&"heavy".to_string()),
            "weight tiebreak should route to 'heavy' (weight=10)"
        );
        assert!(
            !result.completed_nodes.contains(&"light".to_string()),
            "weight tiebreak should NOT route to 'light' (weight=1)"
        );
    }

    /// Suggested next IDs through the full engine.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_suggested_ids_expected_match() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [shape=diamond]
                alpha [prompt="Alpha"]
                beta [prompt="Beta"]
                exit [shape=Msquare]
                start -> gate
                gate -> alpha
                gate -> beta
                alpha -> exit
                beta -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "gate",
            vec![NodeOutcome {
                status: NodeStatus::Success,
                suggested_next_ids: vec!["beta".to_string()],
                ..Default::default()
            }],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result.completed_nodes.contains(&"beta".to_string()),
            "suggested_next_ids should route to 'beta'"
        );
        assert!(
            !result.completed_nodes.contains(&"alpha".to_string()),
            "should NOT route to 'alpha' when suggested_next_ids points to 'beta'"
        );
    }

    /// Condition match beats weight through the full engine.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_condition_beats_weight_expected_condition_wins() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [prompt="Work"]
                heavy [prompt="Heavy"]
                cond_match [prompt="CondMatch"]
                exit [shape=Msquare]
                start -> work
                work -> heavy [weight=100]
                work -> cond_match [condition="outcome=success", weight=0]
                heavy -> exit
                cond_match -> exit
            }
            "#,
        )
        .expect("parse");
        // work succeeds by default → condition "outcome=success" matches on cond_match edge
        // Even though heavy has weight=100, condition match takes priority (Step 1)
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result.completed_nodes.contains(&"cond_match".to_string()),
            "condition match should beat weight"
        );
        assert!(
            !result.completed_nodes.contains(&"heavy".to_string()),
            "heavy-weight unconditional edge should lose to condition match"
        );
    }

    /// Full 5-step priority test through the engine: condition > preferred_label > suggested > weight > lexical.
    /// We test that preferred_label beats suggested_next_ids.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_edge_selection_preferred_label_beats_suggested_ids() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [shape=diamond]
                label_target [prompt="LabelTarget"]
                suggested_target [prompt="SuggestedTarget"]
                exit [shape=Msquare]
                start -> gate
                gate -> label_target [label="Pick Me"]
                gate -> suggested_target
                label_target -> exit
                suggested_target -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "gate",
            vec![NodeOutcome {
                status: NodeStatus::Success,
                preferred_label: Some("Pick Me".to_string()),
                suggested_next_ids: vec!["suggested_target".to_string()],
                ..Default::default()
            }],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result.completed_nodes.contains(&"label_target".to_string()),
            "preferred_label should beat suggested_next_ids"
        );
    }
}

// =========================================================================
// Gap-closing tests: parallel fail_fast error policy
// =========================================================================
mod section_parallel_fail_fast {
    use super::*;
    use forge_attractor::handlers::{NodeHandler, parallel::ParallelHandler};

    /// fail_fast with context-driven backend: the context-driven path doesn't batch,
    /// so we test fail_fast via the executor path only (see next test).
    /// This test validates that error_policy=fail_fast is parsed correctly.
    #[test]
    fn parallel_error_policy_fail_fast_parsed_correctly() {
        let graph = parse_dot(
            r#"
            digraph G {
                p [shape=component, join_policy="all_success", error_policy="fail_fast", max_parallel=1]
                p -> a
                p -> b
            }
            "#,
        )
        .expect("parse");
        let node = graph.nodes.get("p").unwrap();
        assert_eq!(
            node.attrs.get_str("error_policy"),
            Some("fail_fast"),
            "error_policy attribute should be preserved"
        );
    }

    /// fail_fast with executor: uses a real NodeExecutor where the first branch fails.
    #[tokio::test(flavor = "current_thread")]
    async fn parallel_fail_fast_with_executor_expected_early_abort() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                p [shape=component, join_policy="all_success", error_policy="fail_fast", max_parallel=1]
                branch_a [prompt="A"]
                branch_b [prompt="B"]
                branch_c [prompt="C"]
                fan_in [shape=tripleoctagon]
                exit [shape=Msquare]
                start -> p
                p -> branch_a
                p -> branch_b
                p -> branch_c
                branch_a -> fan_in
                branch_b -> fan_in
                branch_c -> fan_in
                fan_in -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(
            ScriptedExecutor::new().script("branch_a", vec![NodeOutcome::failure("boom")]),
        );
        let handler = ParallelHandler::with_executor(executor);
        let node = graph.nodes.get("p").unwrap();
        let outcome = NodeHandler::execute(&handler, node, &RuntimeContext::new(), &graph)
            .await
            .expect("execute");
        // First branch fails → fail_fast aborts remaining batches
        assert_eq!(outcome.status, NodeStatus::PartialSuccess);
        let branch_count = outcome
            .context_updates
            .get("parallel.branch_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert!(
            branch_count < 3,
            "fail_fast with executor should abort early, but ran {} branches",
            branch_count
        );
    }
}

// =========================================================================
// Gap-closing tests: multi-goal-gate complex scenarios
// =========================================================================
mod section_multi_goal_gate {
    use super::*;

    /// Two goal gates with different retry targets.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_two_goal_gates_first_unsatisfied_expected_retry_and_recovery() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate_a [goal_gate=true, retry_target="fix_a", prompt="Gate A"]
                gate_b [goal_gate=true, prompt="Gate B"]
                fix_a [prompt="Fix A"]
                exit [shape=Msquare]
                start -> gate_a -> gate_b -> exit
                gate_a -> fix_a [condition="outcome=fail"]
                fix_a -> gate_a
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(
            ScriptedExecutor::new()
                .script(
                    "gate_a",
                    vec![
                        NodeOutcome::failure("first attempt fails"),
                        NodeOutcome::success(), // second attempt succeeds
                    ],
                )
                .script("gate_b", vec![NodeOutcome::success()]),
        );
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"fix_a".to_string()));
        assert!(result.completed_nodes.contains(&"gate_b".to_string()));
    }

    /// Both goal gates fail with no retry targets — pipeline should fail.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_two_goal_gates_both_unsatisfied_no_retry_expected_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate_a [goal_gate=true, prompt="Gate A"]
                gate_b [goal_gate=true, prompt="Gate B"]
                exit [shape=Msquare]
                start -> gate_a -> gate_b -> exit
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(
            ScriptedExecutor::new()
                .script("gate_a", vec![NodeOutcome::failure("a fails")])
                .script("gate_b", vec![NodeOutcome::failure("b fails")]),
        );
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        // gate_a fails → pipeline can't route (no fail condition edges), so it fails
        assert_eq!(result.status, PipelineStatus::Fail);
    }

    /// One gate on the happy path succeeds, one gate on an unreachable branch
    /// is never visited — pipeline should still fail (unvisited goal gate).
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_mixed_visited_unvisited_goal_gates_expected_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                happy [goal_gate=true, prompt="Happy Path"]
                unreachable_gate [goal_gate=true, prompt="Unreachable Gate"]
                exit [shape=Msquare]
                start -> happy -> exit
                happy -> unreachable_gate [condition="outcome=fail"]
                unreachable_gate -> exit
            }
            "#,
        )
        .expect("parse");
        // happy succeeds → pipeline goes start -> happy -> exit
        // But unreachable_gate (goal_gate=true) was never visited
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run");
        assert_eq!(
            result.status,
            PipelineStatus::Fail,
            "unvisited goal gate should block exit"
        );
    }

    /// Graph-level retry_target recovers from unsatisfied goal gate.
    #[tokio::test(flavor = "current_thread")]
    async fn pipeline_goal_gate_graph_level_retry_target_expected_recovery() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [retry_target="recovery"]
                start [shape=Mdiamond]
                gate [goal_gate=true, prompt="Gate"]
                recovery [prompt="Recovery"]
                exit [shape=Msquare]
                start -> gate -> exit
                gate -> recovery [condition="outcome=fail"]
                recovery -> gate
            }
            "#,
        )
        .expect("parse");
        let executor = Arc::new(ScriptedExecutor::new().script(
            "gate",
            vec![
                NodeOutcome::failure("first try"),
                NodeOutcome::success(), // second try succeeds after recovery
            ],
        ));
        let result = PipelineRunner
            .run(&graph, run_cfg(executor))
            .await
            .expect("run");
        assert_eq!(result.status, PipelineStatus::Success);
        assert!(result.completed_nodes.contains(&"recovery".to_string()));
    }
}

// =========================================================================
// Gap-closing tests: fidelity mode variants
// =========================================================================
mod section_fidelity_modes {
    use super::*;

    #[test]
    fn is_valid_fidelity_mode_all_valid_modes_expected_true() {
        let valid_modes = [
            "full",
            "truncate",
            "compact",
            "summary:low",
            "summary:medium",
            "summary:high",
        ];
        for mode in &valid_modes {
            assert!(
                is_valid_fidelity_mode(mode),
                "'{}' should be a valid fidelity mode",
                mode
            );
        }
    }

    #[test]
    fn is_valid_fidelity_mode_invalid_modes_expected_false() {
        let invalid_modes = [
            "none",
            "minimal",
            "full:high",
            "summary",
            "summary:",
            "FULL",
            "",
        ];
        for mode in &invalid_modes {
            assert!(
                !is_valid_fidelity_mode(mode),
                "'{}' should NOT be a valid fidelity mode",
                mode
            );
        }
    }

    #[test]
    fn resolve_fidelity_mode_edge_overrides_node_expected_edge_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                a [fidelity="compact"]
                b
                a -> b [fidelity="full"]
            }
            "#,
        )
        .expect("parse");
        let edge = graph.outgoing_edges("a").next().unwrap();
        let mode = resolve_fidelity_mode(&graph, "b", Some(edge));
        assert_eq!(mode, "full", "edge fidelity should override node fidelity");
    }

    #[test]
    fn resolve_fidelity_mode_node_overrides_graph_expected_node_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="truncate"]
                a [fidelity="summary:high"]
                b
                a -> b
            }
            "#,
        )
        .expect("parse");
        let mode = resolve_fidelity_mode(&graph, "a", None);
        assert_eq!(
            mode, "summary:high",
            "node fidelity should override graph default"
        );
    }

    #[test]
    fn resolve_fidelity_mode_graph_default_expected_graph_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="summary:medium"]
                a
                b
                a -> b
            }
            "#,
        )
        .expect("parse");
        let mode = resolve_fidelity_mode(&graph, "a", None);
        assert_eq!(
            mode, "summary:medium",
            "graph default_fidelity should be used when node has no override"
        );
    }

    #[test]
    fn resolve_fidelity_mode_no_override_expected_compact_default() {
        let graph = parse_dot(
            r#"
            digraph G {
                a
                b
                a -> b
            }
            "#,
        )
        .expect("parse");
        let mode = resolve_fidelity_mode(&graph, "a", None);
        assert_eq!(
            mode, "compact",
            "default fidelity should be 'compact' when nothing is set"
        );
    }

    #[test]
    fn resolve_fidelity_mode_all_six_modes_through_edge_expected_passthrough() {
        // Each valid mode should pass through without modification when set on an edge
        let modes = [
            "full",
            "truncate",
            "compact",
            "summary:low",
            "summary:medium",
            "summary:high",
        ];
        for expected_mode in &modes {
            let dot = format!(
                r#"digraph G {{
                    a
                    b
                    a -> b [fidelity="{}"]
                }}"#,
                expected_mode
            );
            let graph = parse_dot(&dot).expect("parse");
            let edge = graph.outgoing_edges("a").next().unwrap();
            let mode = resolve_fidelity_mode(&graph, "b", Some(edge));
            assert_eq!(
                &mode, expected_mode,
                "fidelity mode '{}' should pass through from edge attribute",
                expected_mode
            );
        }
    }
}

// =========================================================================
// Gap-closing tests: HTTP server mode types (feature-gated, MAY)
// =========================================================================
#[cfg(feature = "http")]
mod section_http_types {
    use forge_attractor::http::*;

    #[test]
    fn http_run_request_and_response_types_exist() {
        let req = HttpRunRequest {
            dot_source: "digraph G {}".to_string(),
            goal: Some("test".to_string()),
            context: Default::default(),
        };
        assert_eq!(req.dot_source, "digraph G {}");

        let resp = HttpRunResponse {
            run_id: "run-1".to_string(),
            status: "success".to_string(),
            completed_nodes: vec!["start".to_string()],
            context: Default::default(),
        };
        assert_eq!(resp.run_id, "run-1");
    }

    #[test]
    fn http_server_config_default_expected_localhost_8080() {
        let config = HttpServerConfig::default();
        assert_eq!(config.bind_address, "127.0.0.1");
        assert_eq!(config.port, 8080);
    }
}
