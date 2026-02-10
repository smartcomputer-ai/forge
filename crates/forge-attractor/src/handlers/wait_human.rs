use crate::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, RuntimeContext, handlers::NodeHandler,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub use crate::interviewer::{
    AutoApproveInterviewer, CallbackInterviewer, ConsoleInterviewer, HumanAnswer, HumanChoice,
    HumanQuestion, HumanQuestionType, Interviewer, QueueInterviewer, RecordedInterview,
    RecordingInterviewer,
};

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

        let default_choice = resolve_default_choice(node, &choices).map(|choice| choice.key);
        let question = HumanQuestion {
            stage: node.id.clone(),
            text: node
                .attrs
                .get_str("label")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Select an option:")
                .to_string(),
            question_type: HumanQuestionType::MultipleChoice,
            choices: choices.clone(),
            default_choice: default_choice.clone(),
            timeout: parse_timeout(node),
        };

        let answer = ask_with_timeout(self.interviewer.as_ref(), question.clone()).await;
        let selected = match selected_choice_from_answer(&choices, &answer) {
            Some(choice) => choice,
            None => match answer {
                HumanAnswer::Timeout => {
                    if let Some(default_key) = default_choice {
                        if let Some(choice) = find_choice(&choices, &default_key) {
                            choice
                        } else {
                            return Ok(retry_on_timeout());
                        }
                    } else {
                        return Ok(retry_on_timeout());
                    }
                }
                HumanAnswer::Skipped => {
                    return Ok(NodeOutcome::failure("human skipped interaction"));
                }
                HumanAnswer::No => return Ok(NodeOutcome::failure("human declined interaction")),
                HumanAnswer::FreeText(_) => {
                    return Ok(NodeOutcome::failure(
                        "human free-text did not match a choice",
                    ));
                }
                HumanAnswer::Yes => choices[0].clone(),
                HumanAnswer::Selected(_) => choices[0].clone(),
            },
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

fn retry_on_timeout() -> NodeOutcome {
    NodeOutcome {
        status: NodeStatus::Retry,
        notes: Some("human gate timeout, no default".to_string()),
        context_updates: RuntimeContext::new(),
        preferred_label: None,
        suggested_next_ids: Vec::new(),
    }
}

async fn ask_with_timeout(interviewer: &dyn Interviewer, question: HumanQuestion) -> HumanAnswer {
    let Some(timeout) = question.timeout else {
        return interviewer.ask(question).await;
    };
    match tokio::time::timeout(timeout, interviewer.ask(question)).await {
        Ok(answer) => answer,
        Err(_) => HumanAnswer::Timeout,
    }
}

fn selected_choice_from_answer(
    choices: &[HumanChoice],
    answer: &HumanAnswer,
) -> Option<HumanChoice> {
    match answer {
        HumanAnswer::Selected(raw) => find_choice(choices, raw),
        HumanAnswer::Yes => choices.first().cloned(),
        HumanAnswer::No => None,
        HumanAnswer::FreeText(raw) => find_choice(choices, raw),
        HumanAnswer::Timeout | HumanAnswer::Skipped => None,
    }
}

fn resolve_default_choice(node: &Node, choices: &[HumanChoice]) -> Option<HumanChoice> {
    let raw = attr_str(node, &["human.default_choice"])?;
    find_choice(choices, raw)
}

fn parse_timeout(node: &Node) -> Option<Duration> {
    for key in attr_key_variants("human.timeout_seconds") {
        let Some(value) = node.attrs.get(&key) else {
            continue;
        };
        let seconds = match value {
            crate::AttrValue::Integer(value) if *value > 0 => *value as f64,
            crate::AttrValue::Float(value) if *value > 0.0 => *value,
            crate::AttrValue::String(value) => value.parse::<f64>().ok().unwrap_or(0.0),
            crate::AttrValue::Duration(value) => {
                return Some(Duration::from_millis(value.millis.max(1)));
            }
            _ => 0.0,
        };
        if seconds > 0.0 {
            let millis = (seconds * 1000.0).round() as u64;
            return Some(Duration::from_millis(millis.max(1)));
        }
    }
    None
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

fn attr_key_variants(key: &str) -> Vec<String> {
    vec![key.to_string(), key.replace('.', "_")]
}

fn attr_str<'a>(node: &'a Node, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(value) = node.attrs.get_str(key) {
            return Some(value);
        }
        let underscored = key.replace('.', "_");
        if let Some(value) = node.attrs.get_str(&underscored) {
            return Some(value);
        }
    }
    None
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

    struct SlowInterviewer;

    #[async_trait]
    impl Interviewer for SlowInterviewer {
        async fn ask(&self, _question: HumanQuestion) -> HumanAnswer {
            tokio::time::sleep(Duration::from_millis(50)).await;
            HumanAnswer::Selected("Y".to_string())
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

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_timeout_with_default_choice_expected_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                gate [shape=hexagon, human_default_choice="Y"]
                yes
                no
                gate -> yes [label="[Y] Yes"]
                gate -> no [label="[N] No"]
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
        assert_eq!(outcome.status, NodeStatus::Success);
        assert_eq!(outcome.suggested_next_ids, vec!["yes".to_string()]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_human_handler_timeout_config_expected_enforced_by_handler() {
        let graph = parse_dot(
            r#"
            digraph G {
                gate [shape=hexagon, human_timeout_seconds=0.01]
                yes
                gate -> yes [label="[Y] Yes"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("gate").expect("gate should exist");
        let handler = WaitHumanHandler::new(Arc::new(SlowInterviewer));

        let outcome = handler
            .execute(node, &RuntimeContext::new(), &graph)
            .await
            .expect("execution should succeed");
        assert_eq!(outcome.status, NodeStatus::Retry);
    }
}
