use crate::{
    AttractorError, AttractorStageToAgentLinkRecord, AttractorStorageWriter, Graph, Node,
    NodeOutcome, NodeStatus, RuntimeContext,
    handlers::codergen::{CodergenBackend, CodergenBackendResult},
};
use async_trait::async_trait;
use forge_agent::{AgentError, Session, SubmitOptions, SubmitResult};
use forge_turnstore::{ContextId, TurnId, attractor_idempotency_key};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[async_trait]
pub trait AgentSubmitter: Send {
    async fn submit_with_result(
        &mut self,
        user_input: String,
        options: SubmitOptions,
    ) -> Result<SubmitResult, AgentError>;

    fn thread_key(&self) -> Option<&str>;

    fn set_thread_key(&mut self, thread_key: Option<String>);

    fn session_id(&self) -> &str;
}

#[async_trait]
impl AgentSubmitter for Session {
    async fn submit_with_result(
        &mut self,
        user_input: String,
        options: SubmitOptions,
    ) -> Result<SubmitResult, AgentError> {
        Session::submit_with_result(self, user_input, options).await
    }

    fn thread_key(&self) -> Option<&str> {
        Session::thread_key(self)
    }

    fn set_thread_key(&mut self, thread_key: Option<String>) {
        Session::set_thread_key(self, thread_key);
    }

    fn session_id(&self) -> &str {
        Session::id(self)
    }
}

#[derive(Clone, Debug)]
pub struct ForgeAgentCodergenAdapter {
    base_options: SubmitOptions,
}

impl Default for ForgeAgentCodergenAdapter {
    fn default() -> Self {
        Self {
            base_options: SubmitOptions::default(),
        }
    }
}

impl ForgeAgentCodergenAdapter {
    pub fn new(base_options: SubmitOptions) -> Self {
        Self { base_options }
    }

    pub fn submit_options_for_node(&self, node: &Node) -> SubmitOptions {
        let mut options = self.base_options.clone();
        if let Some(provider) = node.attrs.get_str("llm_provider") {
            if !provider.trim().is_empty() {
                options.provider = Some(provider.trim().to_string());
            }
        }
        if let Some(model) = node.attrs.get_str("llm_model") {
            if !model.trim().is_empty() {
                options.model = Some(model.trim().to_string());
            }
        }
        if let Some(reasoning) = node.attrs.get_str("reasoning_effort") {
            if !reasoning.trim().is_empty() {
                options.reasoning_effort = Some(reasoning.trim().to_ascii_lowercase());
            }
        }
        options
    }

    pub fn build_prompt(&self, node: &Node, graph: &Graph) -> String {
        let mut prompt = node.attrs.get_str("prompt").unwrap_or_default().to_string();
        if prompt.trim().is_empty() {
            prompt = node
                .attrs
                .get_str("label")
                .filter(|label| !label.trim().is_empty())
                .unwrap_or(node.id.as_str())
                .to_string();
        }
        if let Some(goal) = graph.attrs.get_str("goal") {
            prompt = prompt.replace("$goal", goal);
        }
        prompt
    }

    pub async fn execute_with_submitter(
        &self,
        submitter: &mut (dyn AgentSubmitter + Send),
        node: &Node,
        context: &RuntimeContext,
        graph: &Graph,
        stage_attempt_id: &str,
    ) -> Result<NodeOutcome, AttractorError> {
        submitter.set_thread_key(resolve_thread_key(node, context));

        let prompt = self.build_prompt(node, graph);
        let mut options = self.submit_options_for_node(node);
        options.metadata = Some(stage_metadata(node, stage_attempt_id));

        match submitter.submit_with_result(prompt, options).await {
            Ok(result) => Ok(map_submit_result_to_outcome(
                node,
                submitter.thread_key(),
                result,
            )),
            Err(error) => Ok(NodeOutcome::failure(error.to_string())),
        }
    }

    pub async fn execute_prompt_with_submitter(
        &self,
        submitter: &mut (dyn AgentSubmitter + Send),
        node: &Node,
        context: &RuntimeContext,
        prompt: String,
        stage_attempt_id: &str,
    ) -> Result<NodeOutcome, AttractorError> {
        submitter.set_thread_key(resolve_thread_key(node, context));

        let mut options = self.submit_options_for_node(node);
        options.metadata = Some(stage_metadata(node, stage_attempt_id));

        match submitter.submit_with_result(prompt, options).await {
            Ok(result) => Ok(map_submit_result_to_outcome(
                node,
                submitter.thread_key(),
                result,
            )),
            Err(error) => Ok(NodeOutcome::failure(error.to_string())),
        }
    }
}

pub struct ForgeAgentSessionBackend {
    adapter: ForgeAgentCodergenAdapter,
    submitter: Mutex<Box<dyn AgentSubmitter + Send>>,
}

impl ForgeAgentSessionBackend {
    pub fn new(
        adapter: ForgeAgentCodergenAdapter,
        submitter: Box<dyn AgentSubmitter + Send>,
    ) -> Self {
        Self {
            adapter,
            submitter: Mutex::new(submitter),
        }
    }
}

#[async_trait]
impl CodergenBackend for ForgeAgentSessionBackend {
    async fn run(
        &self,
        node: &Node,
        prompt: &str,
        context: &RuntimeContext,
    ) -> Result<CodergenBackendResult, AttractorError> {
        let stage_attempt_id = context
            .get("stage_attempt_id")
            .and_then(Value::as_str)
            .unwrap_or("attempt:1");
        let mut submitter = self.submitter.lock().await;
        let outcome = self
            .adapter
            .execute_prompt_with_submitter(
                submitter.as_mut(),
                node,
                context,
                prompt.to_string(),
                stage_attempt_id,
            )
            .await?;
        Ok(CodergenBackendResult::Outcome(outcome))
    }
}

pub struct StageLinkEmission<'a> {
    pub writer: Arc<dyn AttractorStorageWriter>,
    pub context_id: &'a ContextId,
    pub run_id: &'a str,
    pub node_id: &'a str,
    pub stage_attempt_id: &'a str,
    pub agent_session_id: &'a str,
    pub agent_context_id: &'a ContextId,
    pub agent_head_turn_id: Option<TurnId>,
    pub parent_turn_id: Option<TurnId>,
    pub sequence_no: u64,
    pub thread_key: Option<String>,
}

pub async fn emit_stage_to_agent_link(
    request: StageLinkEmission<'_>,
) -> Result<(), AttractorError> {
    let record = AttractorStageToAgentLinkRecord {
        timestamp: timestamp_now(),
        run_id: request.run_id.to_string(),
        pipeline_context_id: request.context_id.clone(),
        node_id: request.node_id.to_string(),
        stage_attempt_id: request.stage_attempt_id.to_string(),
        agent_session_id: request.agent_session_id.to_string(),
        agent_context_id: request.agent_context_id.clone(),
        agent_head_turn_id: request.agent_head_turn_id,
        parent_turn_id: request.parent_turn_id,
        sequence_no: request.sequence_no,
        thread_key: request.thread_key,
    };
    let key = attractor_idempotency_key(
        request.run_id,
        request.node_id,
        request.stage_attempt_id,
        "stage_to_agent_link",
        request.sequence_no,
    );
    request
        .writer
        .append_stage_to_agent_link(request.context_id, record, key)
        .await?;
    Ok(())
}

fn stage_metadata(node: &Node, stage_attempt_id: &str) -> HashMap<String, String> {
    HashMap::from([
        ("node_id".to_string(), node.id.clone()),
        ("stage_attempt_id".to_string(), stage_attempt_id.to_string()),
    ])
}

fn resolve_thread_key(node: &Node, context: &RuntimeContext) -> Option<String> {
    if let Some(mode) = context
        .get("internal.fidelity.mode")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if !mode.is_empty() && mode != "full" {
            return None;
        }
    }

    if let Some(thread_key) = context
        .get("internal.fidelity.thread_key")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if !thread_key.is_empty() {
            return Some(thread_key.to_string());
        }
    }

    if let Some(thread_id) = node.attrs.get_str("thread_id") {
        if !thread_id.trim().is_empty() {
            return Some(thread_id.trim().to_string());
        }
    }
    context
        .get("thread_key")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn map_submit_result_to_outcome(
    node: &Node,
    active_thread_key: Option<&str>,
    result: SubmitResult,
) -> NodeOutcome {
    let mut updates = RuntimeContext::new();
    updates.insert("last_stage".to_string(), Value::String(node.id.clone()));
    updates.insert(
        "last_response".to_string(),
        Value::String(truncate(&result.assistant_text, 200)),
    );
    updates.insert(
        "agent.tool_call_count".to_string(),
        Value::Number((result.tool_call_count as u64).into()),
    );
    updates.insert(
        "agent.tool_error_count".to_string(),
        Value::Number((result.tool_error_count as u64).into()),
    );
    if let Some(thread) = active_thread_key.or(result.thread_key.as_deref()) {
        updates.insert("thread_key".to_string(), Value::String(thread.to_string()));
    }

    let status = if result.tool_error_count > 0 {
        NodeStatus::PartialSuccess
    } else {
        NodeStatus::Success
    };
    let notes = if result.tool_error_count > 0 {
        Some(format!(
            "completed with {} tool error(s)",
            result.tool_error_count
        ))
    } else {
        Some(format!("Stage completed: {}", node.id))
    };

    NodeOutcome {
        status,
        notes,
        context_updates: updates,
        preferred_label: None,
        suggested_next_ids: Vec::new(),
    }
}

fn truncate(input: &str, max_len: usize) -> String {
    input.chars().take(max_len).collect()
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
    use crate::{AttractorRunEventRecord, AttractorStageEventRecord, parse_dot};
    use forge_agent::SessionState;
    use forge_turnstore::{StoreContext, StoredTurn, TurnStoreError};
    use serde_json::json;

    struct StubSubmitter {
        thread_key: Option<String>,
        last_input: Option<String>,
        last_options: Option<SubmitOptions>,
        result: SubmitResult,
    }

    #[async_trait]
    impl AgentSubmitter for StubSubmitter {
        async fn submit_with_result(
            &mut self,
            user_input: String,
            options: SubmitOptions,
        ) -> Result<SubmitResult, AgentError> {
            self.last_input = Some(user_input);
            self.last_options = Some(options);
            Ok(self.result.clone())
        }

        fn thread_key(&self) -> Option<&str> {
            self.thread_key.as_deref()
        }

        fn set_thread_key(&mut self, thread_key: Option<String>) {
            self.thread_key = thread_key;
        }

        fn session_id(&self) -> &str {
            "session-1"
        }
    }

    #[derive(Default)]
    struct LinkRecordingWriter {
        calls: std::sync::Mutex<Vec<AttractorStageToAgentLinkRecord>>,
    }

    #[async_trait]
    impl AttractorStorageWriter for LinkRecordingWriter {
        async fn create_run_context(
            &self,
            _base_turn_id: Option<TurnId>,
        ) -> Result<StoreContext, TurnStoreError> {
            Ok(StoreContext {
                context_id: "ctx".to_string(),
                head_turn_id: "0".to_string(),
                head_depth: 0,
            })
        }

        async fn append_run_event(
            &self,
            _context_id: &ContextId,
            _record: AttractorRunEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            Err(TurnStoreError::Unsupported("unused".to_string()))
        }

        async fn append_stage_event(
            &self,
            _context_id: &ContextId,
            _record: AttractorStageEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            Err(TurnStoreError::Unsupported("unused".to_string()))
        }

        async fn append_checkpoint_event(
            &self,
            _context_id: &ContextId,
            _record: crate::AttractorCheckpointEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            Err(TurnStoreError::Unsupported("unused".to_string()))
        }

        async fn append_stage_to_agent_link(
            &self,
            _context_id: &ContextId,
            record: AttractorStageToAgentLinkRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.calls.lock().expect("mutex").push(record);
            Ok(StoredTurn {
                context_id: "ctx".to_string(),
                turn_id: "1".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.link.stage_to_agent".to_string(),
                type_version: 1,
                payload: vec![],
                idempotency_key: None,
                content_hash: None,
            })
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_options_for_node_maps_overrides_expected_fields_set() {
        let graph = parse_dot(
            r#"
            digraph G {
                n1 [llm_provider="openai", llm_model="gpt-5", reasoning_effort="high"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node");
        let adapter = ForgeAgentCodergenAdapter::default();
        let options = adapter.submit_options_for_node(node);
        assert_eq!(options.provider.as_deref(), Some("openai"));
        assert_eq!(options.model.as_deref(), Some("gpt-5"));
        assert_eq!(options.reasoning_effort.as_deref(), Some("high"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_with_submitter_maps_submit_result_expected_partial_with_tool_errors() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [goal="ship"]
                n1 [prompt="do $goal", thread_id="thread-main"]
            }
            "#,
        )
        .expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node");
        let mut submitter = StubSubmitter {
            thread_key: None,
            last_input: None,
            last_options: None,
            result: SubmitResult {
                final_state: SessionState::Idle,
                assistant_text: "done".to_string(),
                tool_call_count: 2,
                tool_call_ids: vec!["a".to_string(), "b".to_string()],
                tool_error_count: 1,
                usage: None,
                thread_key: Some("thread-main".to_string()),
            },
        };
        let adapter = ForgeAgentCodergenAdapter::default();
        let outcome = adapter
            .execute_with_submitter(&mut submitter, node, &RuntimeContext::new(), &graph, "a1")
            .await
            .expect("execution should succeed");

        assert_eq!(outcome.status, NodeStatus::PartialSuccess);
        assert_eq!(submitter.thread_key.as_deref(), Some("thread-main"));
        let metadata = submitter
            .last_options
            .as_ref()
            .and_then(|o| o.metadata.as_ref())
            .cloned()
            .unwrap_or_default();
        assert_eq!(metadata.get("node_id").map(String::as_str), Some("n1"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn forge_agent_session_backend_run_expected_codergen_outcome_variant() {
        let graph = parse_dot("digraph G { n1 [prompt=\"hi\"] }").expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node");
        let submitter = StubSubmitter {
            thread_key: None,
            last_input: None,
            last_options: None,
            result: SubmitResult {
                final_state: SessionState::Idle,
                assistant_text: "done".to_string(),
                tool_call_count: 0,
                tool_call_ids: vec![],
                tool_error_count: 0,
                usage: None,
                thread_key: None,
            },
        };
        let backend = ForgeAgentSessionBackend::new(
            ForgeAgentCodergenAdapter::default(),
            Box::new(submitter),
        );
        let result = backend
            .run(node, "hello", &RuntimeContext::new())
            .await
            .expect("backend run should succeed");
        match result {
            CodergenBackendResult::Outcome(outcome) => {
                assert_eq!(outcome.status, NodeStatus::Success);
            }
            CodergenBackendResult::Text(_) => panic!("expected outcome variant"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn emit_stage_to_agent_link_records_expected_payload() {
        let writer = Arc::new(LinkRecordingWriter::default());
        emit_stage_to_agent_link(StageLinkEmission {
            writer: writer.clone(),
            context_id: &"pipeline-ctx".to_string(),
            run_id: "run-1",
            node_id: "plan",
            stage_attempt_id: "plan:attempt:1",
            agent_session_id: "session-1",
            agent_context_id: &"agent-ctx".to_string(),
            agent_head_turn_id: Some("9".to_string()),
            parent_turn_id: Some("3".to_string()),
            sequence_no: 7,
            thread_key: Some("thread-main".to_string()),
        })
        .await
        .expect("emission should succeed");

        let calls = writer.calls.lock().expect("mutex");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].run_id, "run-1");
        assert_eq!(calls[0].node_id, "plan");
    }

    #[test]
    fn resolve_thread_key_prefers_node_thread_id_expected_node_value() {
        let graph = parse_dot("digraph G { n1 [thread_id=\"t1\"] }").expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node");
        let mut context = RuntimeContext::new();
        context.insert("thread_key".to_string(), json!("ctx-thread"));
        assert_eq!(resolve_thread_key(node, &context).as_deref(), Some("t1"));
    }

    #[test]
    fn resolve_thread_key_non_full_fidelity_expected_none() {
        let graph = parse_dot("digraph G { n1 [thread_id=\"t1\"] }").expect("graph should parse");
        let node = graph.nodes.get("n1").expect("node");
        let mut context = RuntimeContext::new();
        context.insert("internal.fidelity.mode".to_string(), json!("truncate"));
        context.insert("thread_key".to_string(), json!("ctx-thread"));
        assert_eq!(resolve_thread_key(node, &context), None);
    }
}
