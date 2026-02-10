use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanChoice {
    pub key: String,
    pub label: String,
    pub to_node: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanQuestion {
    pub stage: String,
    pub text: String,
    pub choices: Vec<HumanChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HumanAnswer {
    Selected(String),
    Timeout,
    Skipped,
}

#[async_trait]
pub trait Interviewer: Send + Sync {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer;
}

#[derive(Debug, Default)]
pub struct AutoApproveInterviewer;

#[async_trait]
impl Interviewer for AutoApproveInterviewer {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer {
        question
            .choices
            .first()
            .map(|choice| HumanAnswer::Selected(choice.key.clone()))
            .unwrap_or(HumanAnswer::Skipped)
    }
}

pub struct WaitHumanHandler {
    interviewer: Arc<dyn Interviewer>,
}

impl WaitHumanHandler {
    pub fn new(interviewer: Arc<dyn Interviewer>) -> Self {
        Self { interviewer }
    }
}

#[async_trait]
impl NodeHandler for WaitHumanHandler {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        let choices = derive_choices(node, graph);
        if choices.is_empty() {
            return Ok(NodeOutcome::failure("No outgoing edges for human gate"));
        }

        let question = HumanQuestion {
            stage: node.id.clone(),
            text: node
                .attrs
                .get_str("label")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Select an option:")
                .to_string(),
            choices: choices.clone(),
        };

        let answer = self.interviewer.ask(question).await;
        let selected = match answer {
            HumanAnswer::Selected(raw) => {
                find_choice(&choices, &raw).unwrap_or_else(|| choices[0].clone())
            }
            HumanAnswer::Timeout => {
                let default_choice = node
                    .attrs
                    .get_str("human.default_choice")
                    .and_then(|raw| find_choice(&choices, raw));
                if let Some(choice) = default_choice {
                    choice
                } else {
                    return Ok(NodeOutcome {
                        status: NodeStatus::Retry,
                        notes: Some("human gate timeout, no default".to_string()),
                        context_updates: RuntimeContext::new(),
                        preferred_label: None,
                        suggested_next_ids: Vec::new(),
                    });
                }
            }
            HumanAnswer::Skipped => return Ok(NodeOutcome::failure("human skipped interaction")),
        };

        let mut updates = RuntimeContext::new();
        updates.insert(
            "human.gate.selected".to_string(),
            Value::String(selected.key.clone()),
        );
        updates.insert(
            "human.gate.label".to_string(),
            Value::String(selected.label.clone()),
        );

        Ok(NodeOutcome {
            status: NodeStatus::Success,
            notes: Some(format!("human selected {}", selected.key)),
            context_updates: updates,
            preferred_label: Some(selected.label.clone()),
            suggested_next_ids: vec![selected.to_node.clone()],
        })
    }
}

fn derive_choices(node: &Node, graph: &Graph) -> Vec<HumanChoice> {
    graph
        .outgoing_edges(&node.id)
        .map(|edge| {
            let label = edge
                .attrs
                .get_str("label")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&edge.to)
                .to_string();
            HumanChoice {
                key: parse_accelerator_key(&label),
                label,
                to_node: edge.to.clone(),
            }
        })
        .collect()
}

fn parse_accelerator_key(label: &str) -> String {
    let trimmed = label.trim();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|raw| raw.split_once(']'))
    {
        let key = inner.0.trim();
        if !key.is_empty() {
            return key.to_ascii_uppercase();
        }
    }
    if let Some((left, _)) = trimmed.split_once(')') {
        let key = left.trim();
        if key.len() == 1 {
            return key.to_ascii_uppercase();
        }
    }
    if let Some((left, _)) = trimmed.split_once('-') {
        let key = left.trim();
        if key.len() == 1 {
            return key.to_ascii_uppercase();
        }
    }
    trimmed
        .chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "X".to_string())
}

fn find_choice(choices: &[HumanChoice], raw: &str) -> Option<HumanChoice> {
    let needle = raw.trim().to_ascii_lowercase();
    choices
        .iter()
        .find(|choice| {
            choice.key.to_ascii_lowercase() == needle
                || choice.label.to_ascii_lowercase() == needle
                || choice.to_node.to_ascii_lowercase() == needle
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

    struct FixedInterviewer(HumanAnswer);

    #[async_trait]
    impl Interviewer for FixedInterviewer {
        async fn ask(&self, _question: HumanQuestion) -> HumanAnswer {
            self.0.clone()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_selected_expected_success_with_suggested_next() {
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
        .expect("graph should parse");
        let node = graph.nodes.get("gate").expect("gate should exist");
        let handler = WaitHumanHandler::new(Arc::new(FixedInterviewer(HumanAnswer::Selected(
            "N".to_string(),
        ))));

        let outcome = handler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(outcome.suggested_next_ids, vec!["no".to_string()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_timeout_without_default_expected_retry() {
        let graph = parse_dot(
            r#"
            digraph G {
                gate [shape=hexagon]
                yes
                gate -> yes [label="[Y] Yes"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("gate").expect("gate should exist");
        let handler = WaitHumanHandler::new(Arc::new(FixedInterviewer(HumanAnswer::Timeout)));

        let outcome = handler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Retry);
    }
}
