use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Default)]
pub struct ToolHandler;

#[async_trait]
impl NodeHandler for ToolHandler {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let command = node
            .attrs
            .get_str("tool_command")
            .unwrap_or_default()
            .trim();
        if command.is_empty() {
            return Ok(NodeOutcome::failure("No tool_command specified"));
        }

        // If tool_output is pre-set (for testing), use it directly
        if let Some(preset_output) = node.attrs.get_str("tool_output") {
            let mut updates = RuntimeContext::new();
            updates.insert(
                "tool.output".to_string(),
                Value::String(preset_output.to_owned()),
            );
            return Ok(NodeOutcome {
                status: NodeStatus::Success,
                notes: Some(format!("Tool completed: {command}")),
                context_updates: updates,
                ..Default::default()
            });
        }

        // Real shell execution
        let timeout = resolve_tool_timeout(node);
        let child_future = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let child = match child_future {
            Ok(child) => child,
            Err(error) => {
                return Ok(NodeOutcome::failure(format!(
                    "failed to spawn command: {error}"
                )));
            }
        };

        let output_result = match timeout {
            Some(timeout_duration) => {
                match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
                    Ok(result) => result,
                    Err(_) => {
                        return Ok(NodeOutcome::failure("tool command timed out"));
                    }
                }
            }
            None => child.wait_with_output().await,
        };

        let output = match output_result {
            Ok(output) => output,
            Err(error) => {
                return Ok(NodeOutcome::failure(format!(
                    "failed to execute command: {error}"
                )));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined = if stderr.is_empty() {
            stdout.clone()
        } else {
            format!("{stdout}\n{stderr}")
        };

        // Write stdout.txt and stderr.txt to stage directory per spec Section 4.7
        if let Some(logs_root) = context.get("runtime.logs_root").and_then(Value::as_str) {
            let stage_dir = PathBuf::from(logs_root).join(&node.id);
            let _ = std::fs::create_dir_all(&stage_dir);
            let _ = std::fs::write(stage_dir.join("stdout.txt"), &stdout);
            let _ = std::fs::write(stage_dir.join("stderr.txt"), &stderr);
        }

        let mut updates = RuntimeContext::new();
        updates.insert("tool.output".to_string(), Value::String(combined.clone()));
        updates.insert("tool.stdout".to_string(), Value::String(stdout));
        updates.insert("tool.stderr".to_string(), Value::String(stderr));
        updates.insert(
            "tool.exit_code".to_string(),
            Value::Number(output.status.code().unwrap_or(-1).into()),
        );

        if output.status.success() {
            Ok(NodeOutcome {
                status: NodeStatus::Success,
                notes: Some(format!("Tool completed: {command}")),
                context_updates: updates,
                ..Default::default()
            })
        } else {
            Ok(NodeOutcome {
                status: NodeStatus::Fail,
                notes: Some(format!(
                    "Tool failed with exit code {}: {command}",
                    output.status.code().unwrap_or(-1)
                )),
                failure_reason: Some(format!("exit code {}", output.status.code().unwrap_or(-1))),
                context_updates: updates,
                ..Default::default()
            })
        }
    }
}

fn resolve_tool_timeout(node: &Node) -> Option<Duration> {
    for key in &["timeout", "timeout_seconds"] {
        if let Some(value) = node.attrs.get(key) {
            let seconds = match value {
                crate::AttrValue::Integer(v) if *v > 0 => *v as f64,
                crate::AttrValue::Float(v) if *v > 0.0 => *v,
                crate::AttrValue::String(v) => v.parse::<f64>().ok().unwrap_or(0.0),
                crate::AttrValue::Duration(d) => {
                    return Some(Duration::from_millis(d.millis.max(1)));
                }
                _ => 0.0,
            };
            if seconds > 0.0 {
                let millis = (seconds * 1000.0).round() as u64;
                return Some(Duration::from_millis(millis.max(1)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_missing_command_expected_fail() {
        let graph = parse_dot("digraph G { t [shape=parallelogram] }").expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Fail);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_command_expected_success_and_output_update() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="echo hi"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert!(outcome.context_updates.contains_key("tool.output"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_handler_real_execution_expected_stdout_captured() {
        let graph = parse_dot(
            r#"
            digraph G {
                t [shape=parallelogram, tool_command="echo hello_world"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        let output = outcome
            .context_updates
            .get("tool.stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(output.contains("hello_world"));
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
        .expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
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
        .expect("graph should parse");
        let node = graph.nodes.get("t").expect("tool node should exist");
        let outcome = ToolHandler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        let output = outcome
            .context_updates
            .get("tool.output")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(output, "preset");
    }
}
