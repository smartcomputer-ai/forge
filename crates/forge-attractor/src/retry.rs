use crate::{Graph, Node, NodeOutcome, NodeStatus};

#[derive(Clone, Debug, PartialEq)]
pub struct RetryBackoffConfig {
    pub initial_delay_ms: u64,
    pub backoff_factor: f64,
    pub max_delay_ms: u64,
    pub jitter: bool,
}

impl Default for RetryBackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff: RetryBackoffConfig,
}

pub fn build_retry_policy(node: &Node, graph: &Graph, backoff: RetryBackoffConfig) -> RetryPolicy {
    let max_retries = node
        .attrs
        .get("max_retries")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            graph
                .attrs
                .get("default_max_retry")
                .and_then(|value| value.as_i64())
        })
        .unwrap_or(0)
        .max(0) as u32;

    RetryPolicy {
        max_attempts: max_retries + 1,
        backoff,
    }
}

pub fn should_retry_outcome(outcome: &NodeOutcome) -> bool {
    matches!(outcome.status, NodeStatus::Retry | NodeStatus::Fail)
}

pub fn finalize_retry_exhausted(node: &Node) -> NodeOutcome {
    if node.attrs.get_bool("allow_partial") == Some(true) {
        return NodeOutcome {
            status: NodeStatus::PartialSuccess,
            notes: Some("retries exhausted, partial accepted".to_string()),
            context_updates: Default::default(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
        };
    }

    NodeOutcome::failure("max retries exceeded")
}

pub fn delay_for_attempt_ms(attempt: u32, config: &RetryBackoffConfig, jitter_seed: u64) -> u64 {
    let exp = (attempt.saturating_sub(1)) as i32;
    let base = (config.initial_delay_ms as f64) * config.backoff_factor.powi(exp);
    let mut delay = base.min(config.max_delay_ms as f64);
    if config.jitter {
        let factor = jitter_factor(attempt, jitter_seed);
        delay *= factor;
    }
    delay.round().max(0.0) as u64
}

fn jitter_factor(attempt: u32, jitter_seed: u64) -> f64 {
    let mut x = jitter_seed ^ ((attempt as u64) << 32) ^ 0x9E3779B97F4A7C15;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let r = x.wrapping_mul(0x2545F4914F6CDD1D);
    let unit = (r as f64) / (u64::MAX as f64);
    0.5 + unit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_dot;

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
        .expect("graph should parse");
        let node = graph.nodes.get("work").expect("work node should exist");

        let policy = build_retry_policy(node, &graph, RetryBackoffConfig::default());
        assert_eq!(policy.max_attempts, 4);
    }

    #[test]
    fn build_retry_policy_graph_default_expected_fallback_used() {
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
        .expect("graph should parse");
        let node = graph.nodes.get("work").expect("work node should exist");

        let policy = build_retry_policy(node, &graph, RetryBackoffConfig::default());
        assert_eq!(policy.max_attempts, 3);
    }

    #[test]
    fn delay_for_attempt_ms_no_jitter_expected_exponential_sequence() {
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
    fn delay_for_attempt_ms_with_jitter_expected_within_bounds() {
        let config = RetryBackoffConfig {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: true,
        };
        let delay = delay_for_attempt_ms(2, &config, 42);
        assert!((200..=1_200).contains(&delay));
    }
}
