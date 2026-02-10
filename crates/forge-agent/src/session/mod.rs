use crate::{
    AgentError, AssistantTurn, CxdbPersistenceMode, EnvironmentContext, EventData, EventEmitter,
    EventKind, EventStream, ExecutionEnvironment, NoopEventEmitter, ProjectDocument,
    ProviderProfile, SessionConfig, SessionError, SessionEvent, SteeringTurn, ToolCallHook,
    ToolDispatchOptions, ToolError, ToolResultTurn, ToolResultsTurn, Turn, UserTurn,
    truncate_tool_output,
};
use forge_cxdb_runtime::{
    CxdbAppendTurnRequest, CxdbBinaryClient, CxdbClientError, CxdbFsSnapshotCapture,
    CxdbFsSnapshotPolicy, CxdbHttpClient, CxdbRuntimeStore, CxdbStoreContext, CxdbStoredTurn,
    CxdbStoredTurnRef, CxdbTurnId,
};
use forge_llm::{Client, Message, Request, ToolCall, ToolChoice, ToolResult, Usage};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use uuid::Uuid;

mod persistence;
use persistence::*;
mod adapters;
use adapters::*;
mod utils;
use utils::*;
mod persistence_flow;
mod runner;
mod subagents;
mod types;
pub use types::{
    SessionCheckpoint, SessionPersistenceSnapshot, SessionState, SubAgentHandle, SubAgentResult,
    SubAgentStatus, SubmitOptions, SubmitResult,
};
use types::{SubAgentRecord, SubAgentTaskOutput};

#[async_trait::async_trait]
pub trait SessionPersistenceWriter: Send + Sync {
    async fn create_context(
        &self,
        base_turn_id: Option<CxdbTurnId>,
    ) -> Result<CxdbStoreContext, CxdbClientError>;

    async fn append_turn(
        &self,
        request: CxdbAppendTurnRequest,
    ) -> Result<CxdbStoredTurn, CxdbClientError>;

    async fn get_head(&self, context_id: &String) -> Result<CxdbStoredTurnRef, CxdbClientError>;

    async fn capture_upload_workspace(
        &self,
        workspace_root: &Path,
        policy: &CxdbFsSnapshotPolicy,
    ) -> Result<CxdbFsSnapshotCapture, CxdbClientError> {
        let _ = (workspace_root, policy);
        Err(CxdbClientError::Backend(
            "capture_upload_workspace is not supported by this persistence writer".to_string(),
        ))
    }

    async fn attach_fs(
        &self,
        turn_id: &CxdbTurnId,
        fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        let _ = (turn_id, fs_root_hash);
        Err(CxdbClientError::Backend(
            "attach_fs is not supported by this persistence writer".to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl<B, H> SessionPersistenceWriter for CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient + Send + Sync,
    H: CxdbHttpClient + Send + Sync,
{
    async fn create_context(
        &self,
        base_turn_id: Option<CxdbTurnId>,
    ) -> Result<CxdbStoreContext, CxdbClientError> {
        CxdbRuntimeStore::create_context(self, base_turn_id).await
    }

    async fn append_turn(
        &self,
        request: CxdbAppendTurnRequest,
    ) -> Result<CxdbStoredTurn, CxdbClientError> {
        CxdbRuntimeStore::append_turn(self, request).await
    }

    async fn get_head(&self, context_id: &String) -> Result<CxdbStoredTurnRef, CxdbClientError> {
        CxdbRuntimeStore::get_head(self, context_id).await
    }

    async fn capture_upload_workspace(
        &self,
        workspace_root: &Path,
        policy: &CxdbFsSnapshotPolicy,
    ) -> Result<CxdbFsSnapshotCapture, CxdbClientError> {
        CxdbRuntimeStore::capture_upload_workspace(self, workspace_root, policy).await
    }

    async fn attach_fs(
        &self,
        turn_id: &CxdbTurnId,
        fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        CxdbRuntimeStore::attach_fs(self, turn_id, fs_root_hash).await
    }
}

pub struct Session {
    id: String,
    provider_profile: Arc<dyn ProviderProfile>,
    provider_profiles: HashMap<String, Arc<dyn ProviderProfile>>,
    execution_env: Arc<dyn ExecutionEnvironment>,
    history: Vec<Turn>,
    event_emitter: Arc<dyn EventEmitter>,
    config: SessionConfig,
    state: SessionState,
    llm_client: Arc<Client>,
    steering_queue: VecDeque<String>,
    followup_queue: VecDeque<String>,
    subagents: HashMap<String, SubAgentHandle>,
    subagent_records: HashMap<String, SubAgentRecord>,
    subagent_depth: usize,
    abort_requested: Arc<AtomicBool>,
    abort_notify: Arc<Notify>,
    tool_call_hook: Option<Arc<dyn ToolCallHook>>,
    thread_key: Option<String>,
    persistence_writer: Option<Arc<dyn SessionPersistenceWriter>>,
    persistence_context_id: Option<String>,
    persistence_parent_turn_id: Option<String>,
    persistence_sequence_no: u64,
    persistence_mode: CxdbPersistenceMode,
}

#[derive(Clone)]
pub struct SessionAbortHandle {
    abort_requested: Arc<AtomicBool>,
    abort_notify: Arc<Notify>,
}

impl SessionAbortHandle {
    pub fn request_abort(&self) {
        self.abort_requested.store(true, Ordering::SeqCst);
        self.abort_notify.notify_waiters();
    }
}

impl Session {
    pub fn new(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
    ) -> Result<Self, AgentError> {
        Self::new_with_emitter(
            provider_profile,
            execution_env,
            llm_client,
            config,
            Arc::new(NoopEventEmitter),
        )
    }

    pub fn new_with_persistence(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        persistence_writer: Option<Arc<dyn SessionPersistenceWriter>>,
    ) -> Result<Self, AgentError> {
        Self::new_with_emitter_and_persistence(
            provider_profile,
            execution_env,
            llm_client,
            config,
            Arc::new(NoopEventEmitter),
            persistence_writer,
        )
    }

    pub fn new_with_cxdb_persistence(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        binary_client: Arc<dyn CxdbBinaryClient>,
        http_client: Arc<dyn CxdbHttpClient>,
    ) -> Result<Self, AgentError> {
        let runtime_store = Arc::new(CxdbRuntimeStore::new(binary_client, http_client));
        if config.cxdb_persistence == CxdbPersistenceMode::Required {
            publish_agent_registry_bundle_blocking(runtime_store.clone())?;
        }
        let store: Arc<dyn SessionPersistenceWriter> = runtime_store;
        Self::new_with_persistence(
            provider_profile,
            execution_env,
            llm_client,
            config,
            Some(store),
        )
    }

    pub fn new_with_emitter(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
    ) -> Result<Self, AgentError> {
        Self::new_with_emitter_and_persistence(
            provider_profile,
            execution_env,
            llm_client,
            config,
            event_emitter,
            None,
        )
    }

    pub fn new_with_emitter_and_persistence(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        persistence_writer: Option<Arc<dyn SessionPersistenceWriter>>,
    ) -> Result<Self, AgentError> {
        Self::new_with_depth(
            provider_profile,
            execution_env,
            llm_client,
            config,
            event_emitter,
            persistence_writer,
            0,
        )
    }

    fn new_with_depth(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        persistence_writer: Option<Arc<dyn SessionPersistenceWriter>>,
        subagent_depth: usize,
    ) -> Result<Self, AgentError> {
        let persistence_mode = config.cxdb_persistence;
        if persistence_mode == CxdbPersistenceMode::Required && persistence_writer.is_none() {
            return Err(SessionError::InvalidConfiguration(
                "cxdb_persistence=required requires a configured CXDB writer".to_string(),
            )
            .into());
        }
        let thread_key = config.thread_key.clone();
        let mut session = Self {
            id: Uuid::new_v4().to_string(),
            provider_profiles: HashMap::from([(
                provider_profile.id().to_string(),
                provider_profile.clone(),
            )]),
            provider_profile,
            execution_env,
            history: Vec::new(),
            event_emitter,
            config,
            state: SessionState::Idle,
            llm_client,
            steering_queue: VecDeque::new(),
            followup_queue: VecDeque::new(),
            subagents: HashMap::new(),
            subagent_records: HashMap::new(),
            subagent_depth,
            abort_requested: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            tool_call_hook: None,
            thread_key,
            persistence_writer,
            persistence_context_id: None,
            persistence_parent_turn_id: None,
            persistence_sequence_no: 0,
            persistence_mode,
        };
        session.emit(EventKind::SessionStart, EventData::new())?;
        session.persist_session_event_blocking("session_start", serde_json::json!({}))?;
        Ok(session)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn set_state(&mut self, state: SessionState) -> Result<(), AgentError> {
        self.transition_to(state)
    }

    pub fn transition_to(&mut self, next_state: SessionState) -> Result<(), AgentError> {
        if !self.state.can_transition_to(&next_state) {
            return Err(SessionError::InvalidStateTransition {
                from: self.state.to_string(),
                to: next_state.to_string(),
            }
            .into());
        }

        if self.state == next_state {
            return Ok(());
        }

        self.state = next_state;
        if self.state == SessionState::Closed {
            self.close_all_subagents()?;
            self.emit_session_end()?;
        }
        Ok(())
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub fn provider_profile(&self) -> Arc<dyn ProviderProfile> {
        self.provider_profile.clone()
    }

    pub fn register_provider_profile(&mut self, profile: Arc<dyn ProviderProfile>) {
        self.provider_profiles
            .insert(profile.id().to_string(), profile);
    }

    pub fn set_tool_call_hook(&mut self, hook: Option<Arc<dyn ToolCallHook>>) {
        self.tool_call_hook = hook;
    }

    pub fn thread_key(&self) -> Option<&str> {
        self.thread_key.as_deref()
    }

    pub fn set_thread_key(&mut self, thread_key: Option<String>) {
        self.thread_key = thread_key.clone();
        self.config.thread_key = thread_key;
    }

    pub fn execution_env(&self) -> Arc<dyn ExecutionEnvironment> {
        self.execution_env.clone()
    }

    pub fn llm_client(&self) -> Arc<Client> {
        self.llm_client.clone()
    }

    pub fn history(&self) -> &[Turn] {
        &self.history
    }

    pub fn push_turn(&mut self, turn: Turn) {
        self.history.push(turn);
    }

    pub fn steer(&mut self, message: impl Into<String>) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }
        self.steering_queue.push_back(message.into());
        Ok(())
    }

    pub fn follow_up(&mut self, message: impl Into<String>) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }
        self.followup_queue.push_back(message.into());
        Ok(())
    }

    pub fn set_reasoning_effort(
        &mut self,
        reasoning_effort: Option<String>,
    ) -> Result<(), AgentError> {
        if let Some(value) = reasoning_effort.as_deref() {
            validate_reasoning_effort(value)?;
        }
        self.config.reasoning_effort = reasoning_effort.map(|value| value.to_ascii_lowercase());
        Ok(())
    }

    pub fn reasoning_effort(&self) -> Option<&str> {
        self.config.reasoning_effort.as_deref()
    }

    pub fn pop_steering_message(&mut self) -> Option<String> {
        self.steering_queue.pop_front()
    }

    pub fn pop_followup_message(&mut self) -> Option<String> {
        self.followup_queue.pop_front()
    }

    pub fn request_abort(&self) {
        self.abort_handle().request_abort();
    }

    pub fn abort_handle(&self) -> SessionAbortHandle {
        SessionAbortHandle {
            abort_requested: self.abort_requested.clone(),
            abort_notify: self.abort_notify.clone(),
        }
    }

    pub async fn process_input(&mut self, user_input: impl Into<String>) -> Result<(), AgentError> {
        self.submit(user_input).await
    }

    pub async fn submit(&mut self, user_input: impl Into<String>) -> Result<(), AgentError> {
        self.submit_with_options(user_input, SubmitOptions::default())
            .await
    }

    pub async fn submit_with_options(
        &mut self,
        user_input: impl Into<String>,
        options: SubmitOptions,
    ) -> Result<(), AgentError> {
        let mut pending_inputs = VecDeque::from([user_input.into()]);

        while let Some(next_input) = pending_inputs.pop_front() {
            let completed_naturally = self.submit_single(next_input, &options).await?;
            if completed_naturally {
                while let Some(follow_up) = self.pop_followup_message() {
                    pending_inputs.push_back(follow_up);
                }
            }
        }

        Ok(())
    }

    pub async fn submit_with_result(
        &mut self,
        user_input: impl Into<String>,
        options: SubmitOptions,
    ) -> Result<SubmitResult, AgentError> {
        let baseline_turns = self.history.len();
        self.submit_with_options(user_input, options).await?;
        let mut assistant_text = String::new();
        let mut tool_call_count = 0usize;
        let mut tool_call_ids = Vec::new();
        let mut tool_error_count = 0usize;
        let mut usage: Option<Usage> = None;
        for turn in self.history.iter().skip(baseline_turns) {
            match turn {
                Turn::Assistant(turn) => {
                    if !turn.content.is_empty() {
                        assistant_text = turn.content.clone();
                    }
                    tool_call_count += turn.tool_calls.len();
                    tool_call_ids.extend(turn.tool_calls.iter().map(|call| call.id.clone()));
                    usage = Some(match usage.take() {
                        Some(acc) => acc + turn.usage.clone(),
                        None => turn.usage.clone(),
                    });
                }
                Turn::ToolResults(results) => {
                    tool_error_count += results
                        .results
                        .iter()
                        .filter(|result| result.is_error)
                        .count();
                }
                _ => {}
            }
        }

        Ok(SubmitResult {
            final_state: self.state.clone(),
            assistant_text,
            tool_call_count,
            tool_call_ids,
            tool_error_count,
            usage,
            thread_key: self.thread_key.clone(),
        })
    }

    async fn submit_single(
        &mut self,
        user_input: String,
        options: &SubmitOptions,
    ) -> Result<bool, AgentError> {
        if self.state == SessionState::Closed {
            return Err(AgentError::session_closed());
        }

        if self.is_abort_requested() {
            self.shutdown_to_closed().await?;
            return Ok(false);
        }

        let abort_notify = self.abort_notify.clone();
        let abort_requested = self.abort_requested.clone();
        let execution_env = self.execution_env.clone();
        let abort_kill_watchdog = tokio::spawn(async move {
            abort_notify.notified().await;
            if abort_requested.load(Ordering::SeqCst) {
                let _ = execution_env.terminate_all_commands().await;
            }
        });

        self.transition_to(SessionState::Processing)?;
        let user_turn = Turn::User(UserTurn::new(user_input.clone(), current_timestamp()));
        self.push_turn(user_turn.clone());
        self.persist_turn_if_enabled(&user_turn).await?;
        self.emit(
            EventKind::UserInput,
            EventData::from_serializable(serde_json::json!({ "content": user_input }))?,
        )?;
        self.drain_steering_queue().await?;

        let mut round_count = 0usize;
        let mut completed_naturally = false;
        let mut context_warning_emitted = false;
        loop {
            if self.is_abort_requested() {
                abort_kill_watchdog.abort();
                self.shutdown_to_closed().await?;
                return Ok(false);
            }

            if round_count >= self.config.max_tool_rounds_per_input {
                self.event_emitter
                    .emit(SessionEvent::turn_limit_round(self.id.clone(), round_count))?;
                break;
            }

            if self.config.max_turns > 0 && self.history.len() >= self.config.max_turns {
                self.emit(
                    EventKind::TurnLimit,
                    EventData::from_serializable(serde_json::json!({
                        "total_turns": self.history.len()
                    }))?,
                )?;
                break;
            }

            if !context_warning_emitted {
                context_warning_emitted = self.emit_context_usage_warning_if_needed()?;
            }

            let request = self.build_request(options)?;
            self.emit(EventKind::AssistantTextStart, EventData::new())?;
            let response = {
                let llm_client = self.llm_client.clone();
                let llm_call = llm_client.complete(request);
                tokio::pin!(llm_call);
                tokio::select! {
                    result = &mut llm_call => {
                        match result {
                            Ok(response) => response,
                            Err(error) => {
                                self.event_emitter
                                    .emit(SessionEvent::error(self.id.clone(), error.to_string()))?;
                                abort_kill_watchdog.abort();
                                self.shutdown_to_closed().await?;
                                return Err(error.into());
                            }
                        }
                    }
                    _ = self.abort_notify.notified() => {
                        abort_kill_watchdog.abort();
                        self.shutdown_to_closed().await?;
                        return Ok(false);
                    }
                }
            };

            let text = response.text();
            let tool_calls = response.tool_calls();
            let reasoning = response.reasoning();
            if !text.is_empty() {
                self.event_emitter.emit(SessionEvent::assistant_text_delta(
                    self.id.clone(),
                    text.clone(),
                ))?;
            }
            let assistant_turn = Turn::Assistant(AssistantTurn::new(
                text.clone(),
                tool_calls.clone(),
                reasoning.clone(),
                response.usage.clone(),
                Some(response.id),
                current_timestamp(),
            ));
            self.push_turn(assistant_turn.clone());
            self.persist_turn_if_enabled(&assistant_turn).await?;
            self.event_emitter.emit(SessionEvent::assistant_text_end(
                self.id.clone(),
                text.clone(),
                reasoning,
            ))?;

            if tool_calls.is_empty() {
                if should_transition_to_awaiting_input(&text) {
                    self.transition_to(SessionState::AwaitingInput)?;
                } else {
                    completed_naturally = true;
                }
                break;
            }

            round_count += 1;
            let results = self.execute_tool_calls(tool_calls, options).await?;
            let result_turns = results
                .into_iter()
                .map(|result| ToolResultTurn {
                    tool_call_id: result.tool_call_id,
                    content: result.content,
                    is_error: result.is_error,
                })
                .collect();
            let tool_results_turn =
                Turn::ToolResults(ToolResultsTurn::new(result_turns, current_timestamp()));
            self.push_turn(tool_results_turn.clone());
            self.persist_turn_if_enabled(&tool_results_turn).await?;
            self.drain_steering_queue().await?;
            self.inject_loop_detection_warning_if_needed().await?;
        }

        abort_kill_watchdog.abort();
        if self.state == SessionState::Processing {
            self.transition_to(SessionState::Idle)?;
        }
        Ok(completed_naturally)
    }

    async fn execute_tool_calls(
        &mut self,
        tool_calls: Vec<ToolCall>,
        options: &SubmitOptions,
    ) -> Result<Vec<ToolResult>, AgentError> {
        for tool_call in &tool_calls {
            let args = parse_tool_call_arguments(tool_call)?;
            self.persist_event_turn(
                "tool_call_start",
                serde_json::json!({
                    "call_id": tool_call.id,
                    "tool_name": tool_call.name,
                    "arguments": args,
                }),
            )
            .await?;
        }

        let supports_parallel = self
            .resolve_provider_profile(options.provider.as_deref())?
            .capabilities()
            .supports_parallel_tool_calls;
        if tool_calls
            .iter()
            .all(|tool_call| !is_subagent_tool(&tool_call.name))
        {
            let results = self
                .provider_profile
                .tool_registry()
                .dispatch(
                    tool_calls,
                    self.execution_env.clone(),
                    &self.config,
                    self.event_emitter.clone(),
                    ToolDispatchOptions {
                        session_id: self.id.clone(),
                        supports_parallel_tool_calls: supports_parallel,
                        hook: self.tool_call_hook.clone(),
                        hook_strict: self.config.tool_hook_strict,
                    },
                )
                .await?;
            for result in &results {
                self.persist_event_turn(
                    "tool_call_end",
                    serde_json::json!({
                        "call_id": result.tool_call_id.clone(),
                        "is_error": result.is_error,
                        "output": result.content.clone(),
                    }),
                )
                .await?;
            }
            return Ok(results);
        }

        let mut results = Vec::with_capacity(tool_calls.len());
        for tool_call in tool_calls {
            if is_subagent_tool(&tool_call.name) {
                let result = self.execute_subagent_tool_call(tool_call).await?;
                self.persist_event_turn(
                    "tool_call_end",
                    serde_json::json!({
                        "call_id": result.tool_call_id.clone(),
                        "is_error": result.is_error,
                        "output": result.content.clone(),
                    }),
                )
                .await?;
                results.push(result);
                continue;
            }

            let mut standard = self
                .provider_profile
                .tool_registry()
                .dispatch(
                    vec![tool_call],
                    self.execution_env.clone(),
                    &self.config,
                    self.event_emitter.clone(),
                    ToolDispatchOptions {
                        session_id: self.id.clone(),
                        supports_parallel_tool_calls: false,
                        hook: self.tool_call_hook.clone(),
                        hook_strict: self.config.tool_hook_strict,
                    },
                )
                .await?;
            if let Some(result) = standard.pop() {
                self.persist_event_turn(
                    "tool_call_end",
                    serde_json::json!({
                        "call_id": result.tool_call_id.clone(),
                        "is_error": result.is_error,
                        "output": result.content.clone(),
                    }),
                )
                .await?;
                results.push(result);
            }
        }

        Ok(results)
    }

    pub fn close(&mut self) -> Result<(), AgentError> {
        self.transition_to(SessionState::Closed)
    }

    pub fn checkpoint(&self) -> Result<SessionCheckpoint, AgentError> {
        if self
            .subagent_records
            .values()
            .any(|record| record.active_task.is_some())
        {
            return Err(SessionError::CheckpointUnsupported(
                "cannot checkpoint while subagents are still running".to_string(),
            )
            .into());
        }

        Ok(SessionCheckpoint {
            session_id: self.id.clone(),
            state: self.state.clone(),
            history: self.history.clone(),
            steering_queue: self.steering_queue.iter().cloned().collect(),
            followup_queue: self.followup_queue.iter().cloned().collect(),
            config: self.config.clone(),
            thread_key: self.thread_key.clone(),
        })
    }

    pub fn from_checkpoint(
        checkpoint: SessionCheckpoint,
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        event_emitter: Arc<dyn EventEmitter>,
    ) -> Result<Self, AgentError> {
        let mut session = Self::new_with_depth(
            provider_profile.clone(),
            execution_env,
            llm_client,
            checkpoint.config.clone(),
            event_emitter,
            None,
            0,
        )?;
        session.id = checkpoint.session_id;
        session.state = checkpoint.state;
        session.history = checkpoint.history;
        session.steering_queue = VecDeque::from(checkpoint.steering_queue);
        session.followup_queue = VecDeque::from(checkpoint.followup_queue);
        session.config = checkpoint.config;
        session.thread_key = checkpoint.thread_key;
        session.config.thread_key = session.thread_key.clone();
        session.provider_profiles =
            HashMap::from([(provider_profile.id().to_string(), provider_profile)]);
        Ok(session)
    }

    pub fn subagents(&self) -> &HashMap<String, SubAgentHandle> {
        &self.subagents
    }

    pub fn subscribe_events(&self) -> EventStream {
        self.event_emitter.subscribe()
    }

    pub fn emit(&self, kind: EventKind, data: EventData) -> Result<(), AgentError> {
        self.event_emitter
            .emit(SessionEvent::new(kind, self.id.clone(), data))
    }

    fn emit_session_end(&mut self) -> Result<(), AgentError> {
        self.event_emitter.emit(SessionEvent::session_end(
            self.id.clone(),
            self.state.to_string(),
        ))?;
        self.persist_session_event_blocking(
            "session_end",
            serde_json::json!({ "final_state": self.state.to_string() }),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
