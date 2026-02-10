use crate::{Graph, Node};
use forge_agent::{
    AgentError, ToolCallHook, ToolHookContext, ToolPostHookContext, ToolPreHookOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolHookCommands {
    pub pre: Option<String>,
    pub post: Option<String>,
}

impl ToolHookCommands {
    pub fn is_empty(&self) -> bool {
        self.pre.is_none() && self.post.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolHookPhase {
    Pre,
    Post,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolHookEvent {
    pub phase: ToolHookPhase,
    pub timestamp: String,
    pub tool_name: String,
    pub tool_call_id: String,
    pub command: String,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ToolHookSummary {
    pub pre_ok: u32,
    pub pre_skip: u32,
    pub pre_error: u32,
    pub post_ok: u32,
    pub post_non_zero: u32,
    pub post_error: u32,
    pub events: Vec<ToolHookEvent>,
}

#[derive(Clone)]
pub struct ToolHookBridge {
    run_id: String,
    node_id: String,
    stage_attempt_id: String,
    commands: ToolHookCommands,
    events: Arc<Mutex<Vec<ToolHookEvent>>>,
}

impl ToolHookBridge {
    pub fn new(
        run_id: String,
        node_id: String,
        stage_attempt_id: String,
        commands: ToolHookCommands,
    ) -> Self {
        Self {
            run_id,
            node_id,
            stage_attempt_id,
            commands,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn summary(&self) -> ToolHookSummary {
        let events = self.events.lock().expect("tool hook events mutex").clone();
        let mut summary = ToolHookSummary {
            events,
            ..ToolHookSummary::default()
        };
        for event in &summary.events {
            match (event.phase.clone(), event.status.as_str()) {
                (ToolHookPhase::Pre, "ok") => summary.pre_ok += 1,
                (ToolHookPhase::Pre, "skip") => summary.pre_skip += 1,
                (ToolHookPhase::Pre, _) => summary.pre_error += 1,
                (ToolHookPhase::Post, "ok") => summary.post_ok += 1,
                (ToolHookPhase::Post, "non_zero") => summary.post_non_zero += 1,
                (ToolHookPhase::Post, _) => summary.post_error += 1,
            }
        }
        summary
    }

    fn record(&self, event: ToolHookEvent) {
        self.events
            .lock()
            .expect("tool hook events mutex")
            .push(event);
    }

    fn command_input_json(
        &self,
        phase: ToolHookPhase,
        context: &ToolHookContext,
        post: Option<&ToolPostHookContext>,
    ) -> String {
        json!({
            "phase": phase,
            "run_id": self.run_id,
            "node_id": self.node_id,
            "stage_attempt_id": self.stage_attempt_id,
            "session_id": context.session_id,
            "tool_call_id": context.call_id,
            "tool_name": context.tool_name,
            "arguments": context.arguments,
            "post": post.map(|ctx| json!({
                "duration_ms": ctx.duration_ms,
                "output": ctx.output,
                "error": ctx.error,
                "is_error": ctx.is_error,
            })),
        })
        .to_string()
    }
}

#[async_trait::async_trait]
impl ToolCallHook for ToolHookBridge {
    async fn before_tool_call(
        &self,
        context: &ToolHookContext,
    ) -> Result<ToolPreHookOutcome, AgentError> {
        let Some(command) = self.commands.pre.as_ref() else {
            return Ok(ToolPreHookOutcome::Continue);
        };
        let input = self.command_input_json(ToolHookPhase::Pre, context, None);
        match execute_hook(command, &input, self, ToolHookPhase::Pre, context, None) {
            Ok(0) => Ok(ToolPreHookOutcome::Continue),
            Ok(code) => Ok(ToolPreHookOutcome::Skip {
                message: format!("pre-hook skipped tool call (exit code {code})"),
                is_error: false,
            }),
            Err(error) => {
                self.record(ToolHookEvent {
                    phase: ToolHookPhase::Pre,
                    timestamp: timestamp_now(),
                    tool_name: context.tool_name.clone(),
                    tool_call_id: context.call_id.clone(),
                    command: command.clone(),
                    status: "error".to_string(),
                    message: error.clone(),
                });
                Ok(ToolPreHookOutcome::Continue)
            }
        }
    }

    async fn after_tool_call(&self, context: &ToolPostHookContext) -> Result<(), AgentError> {
        let Some(command) = self.commands.post.as_ref() else {
            return Ok(());
        };
        let input = self.command_input_json(ToolHookPhase::Post, &context.tool, Some(context));
        if let Err(error) = execute_hook(
            command,
            &input,
            self,
            ToolHookPhase::Post,
            &context.tool,
            Some(context),
        ) {
            self.record(ToolHookEvent {
                phase: ToolHookPhase::Post,
                timestamp: timestamp_now(),
                tool_name: context.tool.tool_name.clone(),
                tool_call_id: context.tool.call_id.clone(),
                command: command.clone(),
                status: "error".to_string(),
                message: error,
            });
        }
        Ok(())
    }
}

fn execute_hook(
    command: &str,
    stdin_payload: &str,
    bridge: &ToolHookBridge,
    phase: ToolHookPhase,
    context: &ToolHookContext,
    post: Option<&ToolPostHookContext>,
) -> Result<i32, String> {
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .env("FORGE_TOOL_HOOK_PHASE", phase_name(&phase))
        .env("FORGE_RUN_ID", &bridge.run_id)
        .env("FORGE_NODE_ID", &bridge.node_id)
        .env("FORGE_STAGE_ATTEMPT_ID", &bridge.stage_attempt_id)
        .env("FORGE_AGENT_SESSION_ID", &context.session_id)
        .env("FORGE_TOOL_CALL_ID", &context.call_id)
        .env("FORGE_TOOL_NAME", &context.tool_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to launch hook command: {error}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(stdin_payload.as_bytes())
            .map_err(|error| format!("failed writing hook stdin payload: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed waiting for hook command: {error}"))?;
    let status_code = output.status.code().unwrap_or(1);
    let status = if status_code == 0 {
        "ok".to_string()
    } else {
        match phase {
            ToolHookPhase::Pre => "skip".to_string(),
            ToolHookPhase::Post => "non_zero".to_string(),
        }
    };
    let mut message = String::new();
    if !output.stdout.is_empty() {
        message.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !message.is_empty() {
            message.push_str(" | ");
        }
        message.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if message.trim().is_empty() {
        if status_code == 0 {
            message = "hook completed".to_string();
        } else {
            message = format!("hook exited with code {status_code}");
        }
    }
    if let Some(post_context) = post {
        if post_context.is_error && !message.contains("tool_error=true") {
            message.push_str(" | tool_error=true");
        }
    }
    bridge.record(ToolHookEvent {
        phase,
        timestamp: timestamp_now(),
        tool_name: context.tool_name.clone(),
        tool_call_id: context.call_id.clone(),
        command: command.to_string(),
        status,
        message,
    });
    Ok(status_code)
}

fn phase_name(phase: &ToolHookPhase) -> &'static str {
    match phase {
        ToolHookPhase::Pre => "pre",
        ToolHookPhase::Post => "post",
    }
}

fn read_hook_value(node: &Node, graph: &Graph, key: &str) -> Option<String> {
    let underscored_key = key.replace('.', "_");
    let node_value = node
        .attrs
        .get_str(key)
        .or_else(|| node.attrs.get_str(&underscored_key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if node_value.is_some() {
        return node_value;
    }
    graph
        .attrs
        .get_str(key)
        .or_else(|| graph.attrs.get_str(&underscored_key))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn resolve_tool_hook_commands(node: &Node, graph: &Graph) -> ToolHookCommands {
    ToolHookCommands {
        pre: read_hook_value(node, graph, "tool_hooks.pre"),
        post: read_hook_value(node, graph, "tool_hooks.post"),
    }
}

fn timestamp_now() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:03}Z",
        since_epoch.as_secs(),
        since_epoch.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    #[test]
    fn resolve_tool_hook_commands_prefers_node_level_over_graph_level() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [tool_hooks_pre="echo graph-pre", tool_hooks_post="echo graph-post"]
                n1 [tool_hooks_pre="echo node-pre"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node should exist");
        let commands = resolve_tool_hook_commands(node, &graph);

        assert_eq!(commands.pre.as_deref(), Some("echo node-pre"));
        assert_eq!(commands.post.as_deref(), Some("echo graph-post"));
    }
}
