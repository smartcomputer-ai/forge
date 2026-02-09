use crate::{
    AgentError, AssistantTurn, EnvironmentContext, EventData, EventEmitter, EventKind, EventStream,
    ExecutionEnvironment, NoopEventEmitter, ProjectDocument, ProviderProfile, SessionConfig,
    SessionError, SessionEvent, SteeringTurn, ToolDispatchOptions, ToolError, ToolResultTurn,
    ToolResultsTurn, Turn, UserTurn, truncate_tool_output,
};
use forge_llm::{
    Client, ContentPart, Message, Request, Role, ThinkingData, ToolCall, ToolCallData, ToolChoice,
    ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Display};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Idle,
    Processing,
    AwaitingInput,
    Closed,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Processing => "PROCESSING",
            Self::AwaitingInput => "AWAITING_INPUT",
            Self::Closed => "CLOSED",
        }
    }

    pub fn can_transition_to(&self, next: &SessionState) -> bool {
        if self == next {
            return true;
        }

        if *next == SessionState::Closed {
            return true;
        }

        match self {
            SessionState::Idle => matches!(next, SessionState::Processing),
            SessionState::Processing => matches!(
                next,
                SessionState::Processing | SessionState::AwaitingInput | SessionState::Idle
            ),
            SessionState::AwaitingInput => matches!(next, SessionState::Processing),
            SessionState::Closed => false,
        }
    }
}

impl Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubAgentStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubAgentHandle {
    pub id: String,
    pub status: SubAgentStatus,
}

struct SubAgentRecord {
    session: Option<Box<Session>>,
    active_task: Option<tokio::task::JoinHandle<SubAgentTaskOutput>>,
    result: Option<SubAgentResult>,
}

struct SubAgentTaskOutput {
    session: Box<Session>,
    result: SubAgentResult,
}

#[derive(Clone)]
struct ModelOverrideProviderProfile {
    inner: Arc<dyn ProviderProfile>,
    model_override: String,
}

impl ModelOverrideProviderProfile {
    fn new(inner: Arc<dyn ProviderProfile>, model_override: String) -> Self {
        Self {
            inner,
            model_override,
        }
    }
}

impl ProviderProfile for ModelOverrideProviderProfile {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn model(&self) -> &str {
        &self.model_override
    }

    fn tool_registry(&self) -> Arc<crate::ToolRegistry> {
        self.inner.tool_registry()
    }

    fn base_instructions(&self) -> &str {
        self.inner.base_instructions()
    }

    fn project_instruction_files(&self) -> Vec<String> {
        self.inner.project_instruction_files()
    }

    fn build_system_prompt(
        &self,
        environment: &EnvironmentContext,
        tools: &[forge_llm::ToolDefinition],
        project_docs: &[ProjectDocument],
        user_override: Option<&str>,
    ) -> String {
        self.inner
            .build_system_prompt(environment, tools, project_docs, user_override)
    }

    fn tools(&self) -> Vec<forge_llm::ToolDefinition> {
        self.inner.tools()
    }

    fn provider_options(&self) -> Option<Value> {
        self.inner.provider_options()
    }

    fn capabilities(&self) -> crate::ProviderCapabilities {
        self.inner.capabilities()
    }

    fn knowledge_cutoff(&self) -> Option<&str> {
        self.inner.knowledge_cutoff()
    }
}

#[derive(Clone)]
struct ScopedExecutionEnvironment {
    inner: Arc<dyn ExecutionEnvironment>,
    scoped_working_directory: PathBuf,
    platform: String,
    os_version: String,
}

impl ScopedExecutionEnvironment {
    fn new(inner: Arc<dyn ExecutionEnvironment>, scoped_working_directory: PathBuf) -> Self {
        Self {
            platform: inner.platform().to_string(),
            os_version: inner.os_version().to_string(),
            inner,
            scoped_working_directory,
        }
    }

    fn resolve_path(&self, path: &str) -> String {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_string_lossy().to_string()
        } else {
            self.scoped_working_directory
                .join(candidate)
                .to_string_lossy()
                .to_string()
        }
    }
}

#[async_trait::async_trait]
impl ExecutionEnvironment for ScopedExecutionEnvironment {
    async fn read_file(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<String, AgentError> {
        self.inner
            .read_file(&self.resolve_path(path), offset, limit)
            .await
    }

    async fn write_file(&self, path: &str, content: &str) -> Result<(), AgentError> {
        self.inner
            .write_file(&self.resolve_path(path), content)
            .await
    }

    async fn delete_file(&self, path: &str) -> Result<(), AgentError> {
        self.inner.delete_file(&self.resolve_path(path)).await
    }

    async fn move_file(&self, from: &str, to: &str) -> Result<(), AgentError> {
        self.inner
            .move_file(&self.resolve_path(from), &self.resolve_path(to))
            .await
    }

    async fn file_exists(&self, path: &str) -> Result<bool, AgentError> {
        self.inner.file_exists(&self.resolve_path(path)).await
    }

    async fn list_directory(
        &self,
        path: &str,
        depth: usize,
    ) -> Result<Vec<crate::DirEntry>, AgentError> {
        self.inner
            .list_directory(&self.resolve_path(path), depth)
            .await
    }

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<crate::ExecResult, AgentError> {
        let effective_working_dir = working_dir
            .map(|path| self.resolve_path(path))
            .unwrap_or_else(|| self.scoped_working_directory.to_string_lossy().to_string());
        self.inner
            .exec_command(command, timeout_ms, Some(&effective_working_dir), env_vars)
            .await
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: crate::GrepOptions,
    ) -> Result<String, AgentError> {
        self.inner
            .grep(pattern, &self.resolve_path(path), options)
            .await
    }

    async fn glob(&self, pattern: &str, path: &str) -> Result<Vec<String>, AgentError> {
        self.inner.glob(pattern, &self.resolve_path(path)).await
    }

    async fn initialize(&self) -> Result<(), AgentError> {
        self.inner.initialize().await
    }

    async fn cleanup(&self) -> Result<(), AgentError> {
        self.inner.cleanup().await
    }

    async fn terminate_all_commands(&self) -> Result<(), AgentError> {
        self.inner.terminate_all_commands().await
    }

    fn working_directory(&self) -> &Path {
        &self.scoped_working_directory
    }

    fn platform(&self) -> &str {
        &self.platform
    }

    fn os_version(&self) -> &str {
        &self.os_version
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub output: String,
    pub success: bool,
    pub turns_used: usize,
}

pub struct Session {
    id: String,
    provider_profile: Arc<dyn ProviderProfile>,
    execution_env: Arc<dyn ExecutionEnvironment>,
    environment_context: EnvironmentContext,
    project_docs: Vec<ProjectDocument>,
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

    pub fn new_with_emitter(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
    ) -> Result<Self, AgentError> {
        Self::new_with_depth(
            provider_profile,
            execution_env,
            llm_client,
            config,
            event_emitter,
            0,
        )
    }

    fn new_with_depth(
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        llm_client: Arc<Client>,
        config: SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        subagent_depth: usize,
    ) -> Result<Self, AgentError> {
        let environment_context =
            build_environment_context_snapshot(provider_profile.as_ref(), execution_env.as_ref());
        let project_docs = discover_project_documents(
            execution_env.working_directory(),
            provider_profile.as_ref(),
        );

        let session = Self {
            id: Uuid::new_v4().to_string(),
            provider_profile,
            execution_env,
            environment_context,
            project_docs,
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
        };
        session.emit(EventKind::SessionStart, EventData::new())?;
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
        let mut pending_inputs = VecDeque::from([user_input.into()]);

        while let Some(next_input) = pending_inputs.pop_front() {
            let completed_naturally = self.submit_single(next_input).await?;
            if completed_naturally {
                while let Some(follow_up) = self.pop_followup_message() {
                    pending_inputs.push_back(follow_up);
                }
            }
        }

        Ok(())
    }

    async fn submit_single(&mut self, user_input: String) -> Result<bool, AgentError> {
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
        self.push_turn(Turn::User(UserTurn::new(
            user_input.clone(),
            current_timestamp(),
        )));
        self.emit(
            EventKind::UserInput,
            EventData::from_serializable(serde_json::json!({ "content": user_input }))?,
        )?;
        self.drain_steering_queue()?;

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

            let request = self.build_request();
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
            self.push_turn(Turn::Assistant(AssistantTurn::new(
                text.clone(),
                tool_calls.clone(),
                reasoning.clone(),
                response.usage.clone(),
                Some(response.id),
                current_timestamp(),
            )));
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
            let results = self.execute_tool_calls(tool_calls).await?;
            let result_turns = results
                .into_iter()
                .map(|result| ToolResultTurn {
                    tool_call_id: result.tool_call_id,
                    content: result.content,
                    is_error: result.is_error,
                })
                .collect();
            self.push_turn(Turn::ToolResults(ToolResultsTurn::new(
                result_turns,
                current_timestamp(),
            )));
            self.drain_steering_queue()?;
            self.inject_loop_detection_warning_if_needed()?;
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
    ) -> Result<Vec<ToolResult>, AgentError> {
        if tool_calls
            .iter()
            .all(|tool_call| !is_subagent_tool(&tool_call.name))
        {
            return self
                .provider_profile
                .tool_registry()
                .dispatch(
                    tool_calls,
                    self.execution_env.clone(),
                    &self.config,
                    self.event_emitter.clone(),
                    ToolDispatchOptions {
                        session_id: self.id.clone(),
                        supports_parallel_tool_calls: self
                            .provider_profile
                            .capabilities()
                            .supports_parallel_tool_calls,
                    },
                )
                .await;
        }

        let mut results = Vec::with_capacity(tool_calls.len());
        for tool_call in tool_calls {
            if is_subagent_tool(&tool_call.name) {
                results.push(self.execute_subagent_tool_call(tool_call).await?);
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
                    },
                )
                .await?;
            if let Some(result) = standard.pop() {
                results.push(result);
            }
        }

        Ok(results)
    }

    async fn execute_subagent_tool_call(
        &mut self,
        tool_call: ToolCall,
    ) -> Result<ToolResult, AgentError> {
        self.event_emitter.emit(SessionEvent::tool_call_start(
            self.id.clone(),
            tool_call.name.clone(),
            tool_call.id.clone(),
        ))?;

        let arguments = parse_tool_call_arguments(&tool_call)?;
        let output = match tool_call.name.as_str() {
            "spawn_agent" => self.handle_spawn_agent(arguments).await,
            "send_input" => self.handle_send_input(arguments).await,
            "wait" => self.handle_wait(arguments).await,
            "close_agent" => self.handle_close_agent(arguments).await,
            _ => Err(ToolError::UnknownTool(tool_call.name.clone()).into()),
        };

        match output {
            Ok(raw_output) => {
                self.event_emitter.emit(SessionEvent::tool_call_end_output(
                    self.id.clone(),
                    tool_call.id.clone(),
                    raw_output.clone(),
                ))?;
                let truncated = truncate_tool_output(&raw_output, &tool_call.name, &self.config);
                Ok(ToolResult {
                    tool_call_id: tool_call.id,
                    content: Value::String(truncated),
                    is_error: false,
                })
            }
            Err(error) => {
                let message = error.to_string();
                self.event_emitter.emit(SessionEvent::tool_call_end_error(
                    self.id.clone(),
                    tool_call.id.clone(),
                    message.clone(),
                ))?;
                Ok(ToolResult {
                    tool_call_id: tool_call.id,
                    content: Value::String(message),
                    is_error: true,
                })
            }
        }
    }

    async fn handle_spawn_agent(&mut self, arguments: Value) -> Result<String, AgentError> {
        if self.subagent_depth >= self.config.max_subagent_depth {
            return Err(ToolError::Execution(format!(
                "max_subagent_depth={} reached; recursive spawning is blocked",
                self.config.max_subagent_depth
            ))
            .into());
        }

        let task = required_string_argument(&arguments, "task")?;
        let working_dir = optional_string_argument(&arguments, "working_dir")?;
        let model_override = optional_string_argument(&arguments, "model")?;
        let requested_max_turns = optional_usize_argument(&arguments, "max_turns")?;
        let mut child_config = self.config.clone();
        child_config.max_turns = requested_max_turns.unwrap_or(50);
        child_config.max_subagent_depth = self.config.max_subagent_depth;

        let child_execution_env: Arc<dyn ExecutionEnvironment> =
            if let Some(working_dir) = working_dir {
                let scoped_dir = resolve_subagent_working_directory(
                    self.execution_env.working_directory(),
                    &working_dir,
                )?;
                Arc::new(ScopedExecutionEnvironment::new(
                    self.execution_env.clone(),
                    scoped_dir,
                ))
            } else {
                self.execution_env.clone()
            };

        let child_provider_profile: Arc<dyn ProviderProfile> =
            if let Some(model) = model_override.filter(|value| !value.trim().is_empty()) {
                Arc::new(ModelOverrideProviderProfile::new(
                    self.provider_profile.clone(),
                    model,
                ))
            } else {
                self.provider_profile.clone()
            };

        let child_id = Uuid::new_v4().to_string();
        self.subagents.insert(
            child_id.clone(),
            SubAgentHandle {
                id: child_id.clone(),
                status: SubAgentStatus::Running,
            },
        );

        let child_session = Session::new_with_depth(
            child_provider_profile,
            child_execution_env,
            self.llm_client.clone(),
            child_config,
            self.event_emitter.clone(),
            self.subagent_depth + 1,
        )?;
        let active_task = Some(spawn_subagent_submit_task(Box::new(child_session), task));
        self.subagent_records.insert(
            child_id.clone(),
            SubAgentRecord {
                session: None,
                active_task,
                result: None,
            },
        );
        tokio::task::yield_now().await;

        Ok(serde_json::json!({
            "agent_id": child_id,
            "status": subagent_status_label(&SubAgentStatus::Running),
        })
        .to_string())
    }

    async fn handle_send_input(&mut self, arguments: Value) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let message = required_string_argument(&arguments, "message")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        self.reconcile_subagent_record(&agent_id, &mut record, false)
            .await?;

        if record.active_task.is_some() {
            self.subagent_records.insert(agent_id.clone(), record);
            return Err(ToolError::Execution(format!(
                "subagent '{}' is still running; call wait before send_input",
                agent_id
            ))
            .into());
        }

        let Some(session) = record.session.take() else {
            self.subagent_records.insert(agent_id.clone(), record);
            return Err(ToolError::Execution(format!(
                "subagent '{}' is unavailable for new input",
                agent_id
            ))
            .into());
        };

        record.active_task = Some(spawn_subagent_submit_task(session, message));
        self.set_subagent_status(&agent_id, SubAgentStatus::Running);
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": subagent_status_label(&SubAgentStatus::Running),
        })
        .to_string())
    }

    async fn handle_wait(&mut self, arguments: Value) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        self.reconcile_subagent_record(&agent_id, &mut record, true)
            .await?;

        let result = record.result.clone().unwrap_or(SubAgentResult {
            output: String::new(),
            success: matches!(
                self.subagents.get(&agent_id).map(|handle| &handle.status),
                Some(SubAgentStatus::Completed)
            ),
            turns_used: record
                .session
                .as_ref()
                .map(|session| session.history().len())
                .unwrap_or_default(),
        });
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": subagent_status_label(self.subagents.get(&agent_id).map(|h| &h.status).unwrap_or(&SubAgentStatus::Failed)),
            "output": result.output,
            "success": result.success,
            "turns_used": result.turns_used
        })
        .to_string())
    }

    async fn handle_close_agent(&mut self, arguments: Value) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        if let Some(task) = record.active_task.take() {
            task.abort();
        }
        if let Some(session) = record.session.as_mut() {
            session.request_abort();
            let _ = session.close();
        }
        self.set_subagent_status(&agent_id, SubAgentStatus::Failed);
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "closed"
        })
        .to_string())
    }

    async fn reconcile_subagent_record(
        &mut self,
        agent_id: &str,
        record: &mut SubAgentRecord,
        wait_for_completion: bool,
    ) -> Result<(), AgentError> {
        let Some(task) = record.active_task.take() else {
            return Ok(());
        };

        if !wait_for_completion && !task.is_finished() {
            record.active_task = Some(task);
            self.set_subagent_status(agent_id, SubAgentStatus::Running);
            return Ok(());
        }

        match task.await {
            Ok(output) => {
                let status = if output.result.success {
                    SubAgentStatus::Completed
                } else {
                    SubAgentStatus::Failed
                };
                record.session = Some(output.session);
                record.result = Some(output.result);
                self.set_subagent_status(agent_id, status);
            }
            Err(error) => {
                record.result = Some(SubAgentResult {
                    output: format!("subagent task join failed: {}", error),
                    success: false,
                    turns_used: 0,
                });
                self.set_subagent_status(agent_id, SubAgentStatus::Failed);
            }
        }

        Ok(())
    }

    fn set_subagent_status(&mut self, agent_id: &str, status: SubAgentStatus) {
        if let Some(handle) = self.subagents.get_mut(agent_id) {
            handle.status = status;
        }
    }

    pub fn close(&mut self) -> Result<(), AgentError> {
        self.transition_to(SessionState::Closed)
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

    fn emit_session_end(&self) -> Result<(), AgentError> {
        self.event_emitter.emit(SessionEvent::session_end(
            self.id.clone(),
            self.state.to_string(),
        ))
    }

    fn close_all_subagents(&mut self) -> Result<(), AgentError> {
        let agent_ids: Vec<String> = self.subagent_records.keys().cloned().collect();
        for agent_id in agent_ids {
            if let Some(record) = self.subagent_records.get_mut(&agent_id) {
                if let Some(task) = record.active_task.take() {
                    task.abort();
                }
                if let Some(session) = record.session.as_mut() {
                    let _ = session.close();
                }
            }
            self.set_subagent_status(&agent_id, SubAgentStatus::Failed);
        }
        Ok(())
    }

    fn drain_steering_queue(&mut self) -> Result<(), AgentError> {
        while let Some(content) = self.pop_steering_message() {
            self.push_turn(Turn::Steering(SteeringTurn::new(
                content.clone(),
                current_timestamp(),
            )));
            self.event_emitter
                .emit(SessionEvent::steering_injected(self.id.clone(), content))?;
        }
        Ok(())
    }

    fn inject_loop_detection_warning_if_needed(&mut self) -> Result<(), AgentError> {
        if !self.config.enable_loop_detection {
            return Ok(());
        }

        if !detect_loop(&self.history, self.config.loop_detection_window) {
            return Ok(());
        }

        let warning = format!(
            "Loop detected: the last {} tool calls follow a repeating pattern. Try a different approach.",
            self.config.loop_detection_window
        );
        if matches!(
            self.history.last(),
            Some(Turn::Steering(turn)) if turn.content == warning
        ) {
            return Ok(());
        }

        self.push_turn(Turn::Steering(SteeringTurn::new(
            warning.clone(),
            current_timestamp(),
        )));
        self.event_emitter
            .emit(SessionEvent::loop_detection(self.id.clone(), warning))?;
        Ok(())
    }

    fn emit_context_usage_warning_if_needed(&self) -> Result<bool, AgentError> {
        let context_window_size = self.provider_profile.capabilities().context_window_size;
        if context_window_size == 0 {
            return Ok(false);
        }

        let approx_tokens = approximate_context_tokens(&self.history);
        let warning_threshold = context_window_size.saturating_mul(8) / 10;
        if approx_tokens <= warning_threshold {
            return Ok(false);
        }

        let usage_percent = ((approx_tokens as f64 / context_window_size as f64) * 100.0).round();
        self.event_emitter
            .emit(SessionEvent::context_usage_warning(
                self.id.clone(),
                approx_tokens,
                context_window_size,
                usage_percent as usize,
            ))?;
        Ok(true)
    }

    fn build_request(&self) -> Request {
        let tools = self.provider_profile.tools();
        let system_prompt = self.provider_profile.build_system_prompt(
            &self.environment_context,
            &tools,
            &self.project_docs,
            self.config.system_prompt_override.as_deref(),
        );

        let mut messages = vec![Message::system(system_prompt)];
        messages.extend(convert_history_to_messages(&self.history));

        let tools = if tools.is_empty() { None } else { Some(tools) };
        let tool_choice = tools.as_ref().map(|_| ToolChoice {
            mode: "auto".to_string(),
            tool_name: None,
        });

        Request {
            model: self.provider_profile.model().to_string(),
            messages,
            provider: Some(self.provider_profile.id().to_string()),
            tools,
            tool_choice,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort: self.config.reasoning_effort.clone(),
            metadata: None,
            provider_options: self.provider_profile.provider_options(),
        }
    }

    fn is_abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::SeqCst)
    }

    async fn shutdown_to_closed(&mut self) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Ok(());
        }

        let _ = self.execution_env.terminate_all_commands().await;
        self.transition_to(SessionState::Closed)
    }
}

fn is_subagent_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "spawn_agent" | "send_input" | "wait" | "close_agent"
    )
}

fn parse_tool_call_arguments(tool_call: &ToolCall) -> Result<Value, AgentError> {
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        let parsed = serde_json::from_str::<Value>(raw_arguments).map_err(|error| {
            ToolError::Validation(format!(
                "invalid JSON arguments for tool '{}': {}",
                tool_call.name, error
            ))
        })?;
        return Ok(parsed);
    }

    Ok(tool_call.arguments.clone())
}

fn required_string_argument(arguments: &Value, key: &str) -> Result<String, AgentError> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Validation(format!("missing required argument '{}'", key)))?;
    Ok(value.to_string())
}

fn optional_string_argument(arguments: &Value, key: &str) -> Result<Option<String>, AgentError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(ToolError::Validation(format!("argument '{}' must be a string", key)).into());
    };
    Ok(Some(value.to_string()))
}

fn optional_usize_argument(arguments: &Value, key: &str) -> Result<Option<usize>, AgentError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(ToolError::Validation(format!("argument '{}' must be an integer", key)).into());
    };
    Ok(Some(value as usize))
}

fn latest_assistant_output(history: &[Turn]) -> Option<String> {
    history.iter().rev().find_map(|turn| {
        if let Turn::Assistant(assistant) = turn {
            Some(assistant.content.clone())
        } else {
            None
        }
    })
}

fn spawn_subagent_submit_task(
    mut session: Box<Session>,
    input: String,
) -> tokio::task::JoinHandle<SubAgentTaskOutput> {
    tokio::spawn(async move {
        let completion = session.submit(input).await;
        let result = match completion {
            Ok(_) => SubAgentResult {
                output: latest_assistant_output(session.history()).unwrap_or_default(),
                success: true,
                turns_used: session.history().len(),
            },
            Err(error) => SubAgentResult {
                output: error.to_string(),
                success: false,
                turns_used: session.history().len(),
            },
        };
        SubAgentTaskOutput { session, result }
    })
}

fn resolve_subagent_working_directory(
    parent_working_directory: &Path,
    requested: &str,
) -> Result<PathBuf, AgentError> {
    let requested_path = Path::new(requested);
    let candidate = if requested_path.is_absolute() {
        requested_path.to_path_buf()
    } else {
        parent_working_directory.join(requested_path)
    };

    let canonical = canonicalize_or_fallback(&candidate);
    if !canonical.exists() || !canonical.is_dir() {
        return Err(ToolError::Execution(format!(
            "subagent working_dir '{}' does not exist or is not a directory",
            requested
        ))
        .into());
    }

    Ok(canonical)
}

fn subagent_status_label(status: &SubAgentStatus) -> &'static str {
    match status {
        SubAgentStatus::Running => "running",
        SubAgentStatus::Completed => "completed",
        SubAgentStatus::Failed => "failed",
    }
}

fn should_transition_to_awaiting_input(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.ends_with('?') {
        return false;
    }

    // Keep the heuristic deterministic: require a short natural-language question.
    let word_count = trimmed
        .split_whitespace()
        .filter(|segment| segment.chars().any(char::is_alphabetic))
        .count();
    word_count >= 3
}

fn convert_history_to_messages(history: &[Turn]) -> Vec<Message> {
    let mut messages = Vec::new();

    for turn in history {
        match turn {
            Turn::User(turn) => messages.push(Message::user(turn.content.clone())),
            Turn::Assistant(turn) => {
                let mut content = Vec::new();
                if !turn.content.is_empty() {
                    content.push(ContentPart::text(turn.content.clone()));
                }

                if let Some(reasoning) = &turn.reasoning {
                    if !reasoning.is_empty() {
                        content.push(ContentPart::thinking(ThinkingData {
                            text: reasoning.clone(),
                            signature: None,
                            redacted: false,
                        }));
                    }
                }

                for tool_call in &turn.tool_calls {
                    content.push(ContentPart::tool_call(ToolCallData {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                        r#type: "function".to_string(),
                    }));
                }

                if content.is_empty() {
                    content.push(ContentPart::text(String::new()));
                }

                messages.push(Message {
                    role: Role::Assistant,
                    content,
                    name: None,
                    tool_call_id: None,
                });
            }
            Turn::ToolResults(turn) => {
                for result in &turn.results {
                    messages.push(Message::tool_result(
                        result.tool_call_id.clone(),
                        result.content.clone(),
                        result.is_error,
                    ));
                }
            }
            Turn::System(turn) => messages.push(Message::system(turn.content.clone())),
            Turn::Steering(turn) => messages.push(Message::user(turn.content.clone())),
        }
    }

    messages
}

fn current_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

fn current_date_yyyy_mm_dd() -> String {
    #[cfg(windows)]
    let command = ("cmd", vec!["/C", "echo %date%"]);
    #[cfg(not(windows))]
    let command = ("date", vec!["+%Y-%m-%d"]);

    let output = std::process::Command::new(command.0)
        .args(command.1)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !text.is_empty() {
                return text;
            }
        }
    }

    "1970-01-01".to_string()
}

fn build_environment_context_snapshot(
    provider_profile: &dyn ProviderProfile,
    execution_env: &dyn ExecutionEnvironment,
) -> EnvironmentContext {
    let working_directory = canonicalize_or_fallback(execution_env.working_directory());
    let repository_root = find_git_repository_root(&working_directory);
    let (git_branch, git_status_summary, git_recent_commits) = if let Some(root) = &repository_root
    {
        (
            git_current_branch(root),
            git_status_summary(root),
            git_recent_commits(root, 5),
        )
    } else {
        (None, None, Vec::new())
    };

    EnvironmentContext {
        working_directory: working_directory.to_string_lossy().to_string(),
        repository_root: repository_root
            .as_ref()
            .map(|root| root.to_string_lossy().to_string()),
        platform: execution_env.platform().to_string(),
        os_version: execution_env.os_version().to_string(),
        is_git_repository: repository_root.is_some(),
        git_branch,
        git_status_summary,
        git_recent_commits,
        date_yyyy_mm_dd: current_date_yyyy_mm_dd(),
        model: provider_profile.model().to_string(),
        knowledge_cutoff: provider_profile.knowledge_cutoff().map(str::to_string),
    }
}

fn discover_project_documents(
    working_directory: &Path,
    provider_profile: &dyn ProviderProfile,
) -> Vec<ProjectDocument> {
    const PROJECT_DOC_BYTE_BUDGET: usize = 32 * 1024;
    let working_directory = canonicalize_or_fallback(working_directory);
    let root =
        find_git_repository_root(&working_directory).unwrap_or_else(|| working_directory.clone());
    let directories = path_chain_from_root_to_cwd(&root, &working_directory);
    let instruction_files = provider_profile.project_instruction_files();

    let mut docs = Vec::new();
    for directory in directories {
        for instruction_file in &instruction_files {
            let candidate = directory.join(instruction_file);
            if !candidate.is_file() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&candidate) else {
                continue;
            };
            let relative = candidate
                .strip_prefix(&root)
                .unwrap_or(&candidate)
                .to_string_lossy()
                .replace('\\', "/");
            docs.push(ProjectDocument {
                path: relative,
                content,
            });
        }
    }

    truncate_project_documents_to_budget(docs, PROJECT_DOC_BYTE_BUDGET)
}

fn truncate_project_documents_to_budget(
    docs: Vec<ProjectDocument>,
    byte_budget: usize,
) -> Vec<ProjectDocument> {
    let total_bytes: usize = docs
        .iter()
        .map(|document| document.content.as_bytes().len())
        .sum();
    if total_bytes <= byte_budget {
        return docs;
    }

    let mut used = 0usize;
    let mut truncated_docs = Vec::new();
    for document in docs {
        if used >= byte_budget {
            break;
        }

        let document_bytes = document.content.as_bytes().len();
        if used + document_bytes <= byte_budget {
            used += document_bytes;
            truncated_docs.push(document);
            continue;
        }

        let remaining = byte_budget.saturating_sub(used);
        let visible = truncate_str_to_byte_limit(&document.content, remaining);
        let content = if visible.is_empty() {
            crate::profiles::PROJECT_DOC_TRUNCATION_MARKER.to_string()
        } else {
            format!(
                "{}\n{}",
                visible,
                crate::profiles::PROJECT_DOC_TRUNCATION_MARKER
            )
        };
        truncated_docs.push(ProjectDocument {
            path: document.path,
            content,
        });
        break;
    }

    truncated_docs
}

fn truncate_str_to_byte_limit(input: &str, max_bytes: usize) -> String {
    if input.as_bytes().len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    let mut end = max_bytes.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

fn find_git_repository_root(start: &Path) -> Option<PathBuf> {
    let canonical = canonicalize_or_fallback(start);
    for ancestor in canonical.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn path_chain_from_root_to_cwd(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let root = canonicalize_or_fallback(root);
    let cwd = canonicalize_or_fallback(cwd);
    if root == cwd {
        return vec![cwd];
    }
    if !cwd.starts_with(&root) {
        return vec![cwd];
    }

    let mut chain = Vec::new();
    let mut current = cwd.as_path();
    loop {
        chain.push(current.to_path_buf());
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            return vec![cwd];
        };
        current = parent;
    }
    chain.reverse();
    chain
}

fn canonicalize_or_fallback(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn git_current_branch(repository_root: &Path) -> Option<String> {
    run_git_command(repository_root, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn git_status_summary(repository_root: &Path) -> Option<String> {
    let output = run_git_command(repository_root, &["status", "--porcelain"])?;
    let mut modified = 0usize;
    let mut untracked = 0usize;
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with("??") {
            untracked += 1;
        } else {
            modified += 1;
        }
    }
    Some(format!("modified: {modified}, untracked: {untracked}"))
}

fn git_recent_commits(repository_root: &Path, limit: usize) -> Vec<String> {
    run_git_command(
        repository_root,
        &["log", "--oneline", "-n", &limit.to_string()],
    )
    .map(|output| {
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn run_git_command(repository_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Some(String::new());
    }
    Some(text)
}

fn validate_reasoning_effort(value: &str) -> Result<(), AgentError> {
    let normalized = value.to_ascii_lowercase();
    match normalized.as_str() {
        "low" | "medium" | "high" => Ok(()),
        _ => Err(SessionError::InvalidConfiguration(format!(
            "reasoning_effort must be one of: low, medium, high (received '{}')",
            value
        ))
        .into()),
    }
}

fn detect_loop(history: &[Turn], window_size: usize) -> bool {
    if window_size == 0 {
        return false;
    }

    let signatures: Vec<u64> = history
        .iter()
        .filter_map(|turn| {
            if let Turn::Assistant(turn) = turn {
                Some(
                    turn.tool_calls
                        .iter()
                        .map(tool_call_signature)
                        .collect::<Vec<u64>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect();

    if signatures.len() < window_size {
        return false;
    }

    let recent = &signatures[signatures.len() - window_size..];
    for pattern_len in 1..=3 {
        if window_size % pattern_len != 0 {
            continue;
        }

        let pattern = &recent[0..pattern_len];
        let mut all_match = true;
        for chunk in recent.chunks(pattern_len).skip(1) {
            if chunk != pattern {
                all_match = false;
                break;
            }
        }
        if all_match {
            return true;
        }
    }

    false
}

fn tool_call_signature(tool_call: &forge_llm::ToolCall) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_call.name.hash(&mut hasher);
    if let Ok(serialized) = serde_json::to_string(&tool_call.arguments) {
        serialized.hash(&mut hasher);
    } else {
        tool_call.arguments.to_string().hash(&mut hasher);
    }
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        raw_arguments.hash(&mut hasher);
    }
    hasher.finish()
}

fn approximate_context_tokens(history: &[Turn]) -> usize {
    total_chars_in_history(history) / 4
}

fn total_chars_in_history(history: &[Turn]) -> usize {
    history
        .iter()
        .map(|turn| match turn {
            Turn::User(turn) => turn.content.chars().count(),
            Turn::Assistant(turn) => {
                let mut chars = turn.content.chars().count();
                if let Some(reasoning) = &turn.reasoning {
                    chars += reasoning.chars().count();
                }
                for tool_call in &turn.tool_calls {
                    chars += tool_call.id.chars().count();
                    chars += tool_call.name.chars().count();
                    chars += tool_call.arguments.to_string().chars().count();
                    if let Some(raw) = &tool_call.raw_arguments {
                        chars += raw.chars().count();
                    }
                }
                chars
            }
            Turn::ToolResults(turn) => turn
                .results
                .iter()
                .map(|result| {
                    result.tool_call_id.chars().count() + result.content.to_string().chars().count()
                })
                .sum(),
            Turn::System(turn) => turn.content.chars().count(),
            Turn::Steering(turn) => turn.content.chars().count(),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BufferedEventEmitter, LocalExecutionEnvironment, PROJECT_DOC_TRUNCATION_MARKER,
        ProviderCapabilities, RegisteredTool, StaticProviderProfile, ToolExecutor, ToolRegistry,
        build_openai_tool_registry,
    };
    use async_trait::async_trait;
    use forge_llm::{
        Client, ConfigurationError, ContentPart, FinishReason, Message, ProviderAdapter, Request,
        Response, SDKError, StreamEventStream, ToolCallData, Usage,
    };
    use futures::{StreamExt, executor::block_on};
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[derive(Clone)]
    struct SequenceAdapter {
        responses: Arc<Mutex<VecDeque<Response>>>,
        requests: Arc<Mutex<Vec<Request>>>,
        delay_ms: u64,
    }

    #[async_trait]
    impl ProviderAdapter for SequenceAdapter {
        fn name(&self) -> &str {
            "test"
        }

        async fn complete(&self, request: Request) -> Result<Response, SDKError> {
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            self.requests.lock().expect("requests mutex").push(request);
            self.responses
                .lock()
                .expect("responses mutex")
                .pop_front()
                .ok_or_else(|| {
                    SDKError::Configuration(ConfigurationError::new("no response queued"))
                })
        }

        async fn stream(&self, _request: Request) -> Result<StreamEventStream, SDKError> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn test_usage() -> Usage {
        Usage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            raw: None,
        }
    }

    fn text_response(id: &str, text: &str) -> Response {
        Response {
            id: id.to_string(),
            model: "gpt-5.2-codex".to_string(),
            provider: "test".to_string(),
            message: Message::assistant(text),
            finish_reason: FinishReason {
                reason: "stop".to_string(),
                raw: None,
            },
            usage: test_usage(),
            raw: None,
            warnings: Vec::new(),
            rate_limit: None,
        }
    }

    fn tool_call_response(id: &str, call_id: &str, tool_name: &str, args: Value) -> Response {
        Response {
            id: id.to_string(),
            model: "gpt-5.2-codex".to_string(),
            provider: "test".to_string(),
            message: Message {
                role: forge_llm::Role::Assistant,
                content: vec![ContentPart::tool_call(ToolCallData {
                    id: call_id.to_string(),
                    name: tool_name.to_string(),
                    arguments: args,
                    r#type: "function".to_string(),
                })],
                name: None,
                tool_call_id: None,
            },
            finish_reason: FinishReason {
                reason: "tool_calls".to_string(),
                raw: None,
            },
            usage: test_usage(),
            raw: None,
            warnings: Vec::new(),
            rate_limit: None,
        }
    }

    fn build_test_client(responses: Vec<Response>) -> (Arc<Client>, Arc<Mutex<Vec<Request>>>) {
        build_test_client_with_delay(responses, 0)
    }

    fn build_test_client_with_delay(
        responses: Vec<Response>,
        delay_ms: u64,
    ) -> (Arc<Client>, Arc<Mutex<Vec<Request>>>) {
        let adapter = Arc::new(SequenceAdapter {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
            delay_ms,
        });

        let requests = adapter.requests.clone();
        let mut client = Client::default();
        client
            .register_provider(adapter)
            .expect("provider should register");
        (Arc::new(client), requests)
    }

    fn tool_registry_with_echo() -> Arc<ToolRegistry> {
        tool_registry_with_named_echoes(&["echo_tool"])
    }

    fn tool_registry_with_named_echoes(names: &[&str]) -> Arc<ToolRegistry> {
        let mut tool_registry = ToolRegistry::default();
        for name in names {
            let executor: ToolExecutor = Arc::new(|args, _env| {
                Box::pin(async move {
                    let output = args
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or("missing")
                        .to_string();
                    Ok(output)
                })
            });
            tool_registry.register(RegisteredTool {
                definition: forge_llm::ToolDefinition {
                    name: (*name).to_string(),
                    description: "echo".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "required": ["value"],
                        "properties": {
                            "value": { "type": "string" }
                        }
                    }),
                },
                executor,
            });
        }
        Arc::new(tool_registry)
    }

    fn write_test_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directory should be created");
        }
        fs::write(path, content).expect("file should be written");
    }

    #[test]
    fn session_new_emits_session_start() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("session should initialize");

        assert!(!session.id().is_empty());
        let events = emitter.snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::SessionStart);
    }

    #[test]
    fn session_rejects_steer_when_closed() {
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session.close().expect("close should succeed");

        let err = session.steer("halt").expect_err("steer should fail");
        assert!(matches!(err, AgentError::Session(SessionError::Closed)));
    }

    #[test]
    fn session_state_enforces_spec_transitions() {
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .transition_to(SessionState::Processing)
            .expect("idle -> processing should work");
        session
            .transition_to(SessionState::AwaitingInput)
            .expect("processing -> awaiting_input should work");
        session
            .transition_to(SessionState::Processing)
            .expect("awaiting_input -> processing should work");
        session
            .transition_to(SessionState::Idle)
            .expect("processing -> idle should work");

        let err = session
            .transition_to(SessionState::AwaitingInput)
            .expect_err("idle -> awaiting_input should fail");
        assert!(matches!(
            err,
            AgentError::Session(SessionError::InvalidStateTransition { .. })
        ));
    }

    #[test]
    fn closing_session_emits_session_end_once_with_final_state() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let mut session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("session should initialize");

        session.close().expect("close should succeed");
        session.close().expect("second close should be a no-op");

        let events = emitter.snapshot();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, EventKind::SessionStart);
        assert_eq!(events[1].kind, EventKind::SessionEnd);
        assert_eq!(events[1].data.get_str("final_state"), Some("CLOSED"));
    }

    #[test]
    fn session_exposes_async_event_subscription() {
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let client = Arc::new(Client::default());
        let session =
            Session::new_with_emitter(profile, env, client, SessionConfig::default(), emitter)
                .expect("session should initialize");

        let mut stream = session.subscribe_events();
        session
            .emit(
                EventKind::UserInput,
                EventData::from_serializable(serde_json::json!({ "content": "hi" }))
                    .expect("valid object payload"),
            )
            .expect("emit should succeed");

        let first = block_on(stream.next()).expect("session start should arrive");
        assert_eq!(first.kind, EventKind::SessionStart);
        let second = block_on(stream.next()).expect("user input should arrive");
        assert_eq!(second.kind, EventKind::UserInput);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_natural_completion_without_tool_calls_returns_to_idle() {
        let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .submit("hello")
            .await
            .expect("submit should succeed");

        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 2);
        assert!(matches!(session.history()[0], Turn::User(_)));
        assert!(matches!(session.history()[1], Turn::Assistant(_)));
        assert_eq!(requests.lock().expect("requests mutex").len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_transitions_to_awaiting_input_for_question_then_back_to_idle_on_answer() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "Which file should I edit next?"),
            text_response("resp-2", "Done."),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .submit("start")
            .await
            .expect("first submit should succeed");
        assert_eq!(session.state(), &SessionState::AwaitingInput);

        session
            .submit("Edit src/main.rs")
            .await
            .expect("answer submit should succeed");
        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(requests.lock().expect("requests mutex").len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_enforces_per_input_round_limit_and_emits_turn_limit_event() {
        let (client, requests) = build_test_client(vec![
            tool_call_response(
                "resp-1",
                "call-1",
                "echo_tool",
                serde_json::json!({ "value": "first" }),
            ),
            text_response("resp-2", "should_not_be_called"),
        ]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: tool_registry_with_echo(),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut config = SessionConfig::default();
        config.max_tool_rounds_per_input = 1;
        let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
            .expect("new session");

        session
            .submit("run tool")
            .await
            .expect("submit should succeed");

        let events = emitter.snapshot();
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::TurnLimit)
        );
        assert_eq!(requests.lock().expect("requests mutex").len(), 1);
        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 3);
        assert!(matches!(session.history()[2], Turn::ToolResults(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_multiple_times_keeps_history_consistent() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session.submit("one").await.expect("first submit");
        session.submit("two").await.expect("second submit");

        assert_eq!(session.state(), &SessionState::Idle);
        assert_eq!(session.history().len(), 4);
        assert!(matches!(session.history()[0], Turn::User(_)));
        assert!(matches!(session.history()[1], Turn::Assistant(_)));
        assert!(matches!(session.history()[2], Turn::User(_)));
        assert!(matches!(session.history()[3], Turn::Assistant(_)));
        assert_eq!(requests.lock().expect("requests mutex").len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn steering_messages_are_injected_into_history_and_next_request() {
        let (client, requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session
            .steer("Use concise output")
            .expect("steer should queue");

        session
            .submit("hello")
            .await
            .expect("submit should succeed");

        assert!(matches!(session.history()[1], Turn::Steering(_)));
        let requests = requests.lock().expect("requests mutex");
        let first_request = &requests[0];
        assert!(first_request
            .messages
            .iter()
            .any(|message| message.role == Role::User && message.text() == "Use concise output"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn follow_up_queue_triggers_new_processing_cycle_after_completion() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");
        session
            .follow_up("second input")
            .expect("follow-up should queue");

        session
            .submit("first input")
            .await
            .expect("submit should succeed");

        assert_eq!(session.history().len(), 4);
        assert!(matches!(&session.history()[0], Turn::User(turn) if turn.content == "first input"));
        assert!(
            matches!(&session.history()[2], Turn::User(turn) if turn.content == "second input")
        );
        assert_eq!(requests.lock().expect("requests mutex").len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loop_detection_injects_warning_steering_turn_and_event() {
        let (client, requests) = build_test_client(vec![
            tool_call_response(
                "resp-1",
                "call-1",
                "tool_a",
                serde_json::json!({ "value": "a" }),
            ),
            tool_call_response(
                "resp-2",
                "call-2",
                "tool_b",
                serde_json::json!({ "value": "b" }),
            ),
            tool_call_response(
                "resp-3",
                "call-3",
                "tool_a",
                serde_json::json!({ "value": "a" }),
            ),
            tool_call_response(
                "resp-4",
                "call-4",
                "tool_b",
                serde_json::json!({ "value": "b" }),
            ),
            text_response("resp-5", "done"),
        ]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: tool_registry_with_named_echoes(&["tool_a", "tool_b"]),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut config = SessionConfig::default();
        config.loop_detection_window = 4;
        let mut session = Session::new_with_emitter(profile, env, client, config, emitter.clone())
            .expect("new session");

        session
            .submit("start")
            .await
            .expect("submit should succeed");

        assert!(session.history().iter().any(|turn| matches!(
            turn,
            Turn::Steering(turn) if turn.content.contains("Loop detected")
        )));
        assert!(
            emitter
                .snapshot()
                .iter()
                .any(|event| event.kind == EventKind::LoopDetection)
        );

        let requests = requests.lock().expect("requests mutex");
        assert!(requests[4].messages.iter().any(|message| {
            message.role == Role::User && message.text().contains("Loop detected")
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reasoning_effort_updates_apply_to_next_llm_call() {
        let (client, requests) = build_test_client(vec![
            text_response("resp-1", "first"),
            text_response("resp-2", "second"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        session
            .set_reasoning_effort(Some("low".to_string()))
            .expect("low should be valid");
        session.submit("one").await.expect("first submit");
        session
            .set_reasoning_effort(Some("high".to_string()))
            .expect("high should be valid");
        session.submit("two").await.expect("second submit");

        let requests = requests.lock().expect("requests mutex");
        assert_eq!(requests[0].reasoning_effort.as_deref(), Some("low"));
        assert_eq!(requests[1].reasoning_effort.as_deref(), Some("high"));

        let err = session
            .set_reasoning_effort(Some("ultra".to_string()))
            .expect_err("invalid value should be rejected");
        assert!(matches!(
            err,
            AgentError::Session(SessionError::InvalidConfiguration(_))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_emits_context_usage_warning_event_when_history_exceeds_threshold() {
        let (client, _requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities {
                context_window_size: 10,
                ..ProviderCapabilities::default()
            },
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("new session");

        session
            .submit("x".repeat(64))
            .await
            .expect("submit should succeed");

        let events = emitter.snapshot();
        let warning = events
            .iter()
            .find(|event| {
                event.kind == EventKind::Warning
                    && event.data.get_str("category") == Some("context_usage")
            })
            .expect("context usage warning event should be emitted");
        assert_eq!(warning.data.get_str("severity"), Some("warning"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submit_does_not_emit_context_usage_warning_when_usage_is_below_threshold() {
        let (client, _requests) = build_test_client(vec![text_response("resp-1", "done")]);
        let emitter = Arc::new(BufferedEventEmitter::default());
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities {
                context_window_size: 8_000,
                ..ProviderCapabilities::default()
            },
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("new session");

        session.submit("hi").await.expect("submit should succeed");

        let events = emitter.snapshot();
        assert!(!events.iter().any(|event| {
            event.kind == EventKind::Warning
                && event.data.get_str("category") == Some("context_usage")
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn abort_handle_cancels_inflight_llm_call_and_closes_session() {
        let (client, _requests) = build_test_client_with_delay(
            vec![text_response("resp-1", "should not complete normally")],
            2_000,
        );
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: tool_registry_with_echo(),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let emitter = Arc::new(BufferedEventEmitter::default());
        let mut session = Session::new_with_emitter(
            profile,
            env,
            client,
            SessionConfig::default(),
            emitter.clone(),
        )
        .expect("new session");

        let abort_handle = session.abort_handle();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            abort_handle.request_abort();
        });

        let started = std::time::Instant::now();
        session
            .submit("trigger abort")
            .await
            .expect("submit should complete cleanly on abort");

        assert_eq!(session.state(), &SessionState::Closed);
        assert!(started.elapsed() < std::time::Duration::from_millis(800));
        assert!(
            emitter
                .snapshot()
                .iter()
                .any(|event| event.kind == EventKind::SessionEnd),
            "expected SESSION_END after abort"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn abort_handle_terminates_running_shell_command() {
        #[cfg(windows)]
        let command = "ping -n 6 127.0.0.1 > NUL";
        #[cfg(not(windows))]
        let command = "sleep 5";

        let (client, _requests) = build_test_client(vec![tool_call_response(
            "resp-1",
            "call-shell",
            "shell",
            serde_json::json!({ "command": command }),
        )]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(build_openai_tool_registry()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env_dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(env_dir.path()));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        let abort_handle = session.abort_handle();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            abort_handle.request_abort();
        });

        let started = std::time::Instant::now();
        session
            .submit("run long command")
            .await
            .expect("submit should complete after abort");

        assert_eq!(session.state(), &SessionState::Closed);
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    #[test]
    fn discover_project_documents_respects_provider_filter_and_precedence() {
        let tmp = tempdir().expect("temp dir should be created");
        let root = tmp.path();
        let nested = root.join("apps/service");
        fs::create_dir_all(&nested).expect("nested dir should be created");
        fs::create_dir_all(root.join(".git")).expect(".git marker dir should be created");

        write_test_file(&root.join("AGENTS.md"), "root agents");
        write_test_file(&root.join("CLAUDE.md"), "root claude");
        write_test_file(&root.join(".codex/instructions.md"), "root codex");
        write_test_file(&root.join("apps/AGENTS.md"), "apps agents");
        write_test_file(&root.join("apps/CLAUDE.md"), "apps claude");
        write_test_file(&root.join("apps/service/AGENTS.md"), "service agents");

        let profile = StaticProviderProfile {
            id: "anthropic".to_string(),
            model: "claude".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        };

        let docs = discover_project_documents(&nested, &profile);
        let paths: Vec<String> = docs.iter().map(|doc| doc.path.clone()).collect();
        assert_eq!(
            paths,
            vec![
                "AGENTS.md".to_string(),
                "CLAUDE.md".to_string(),
                "apps/AGENTS.md".to_string(),
                "apps/CLAUDE.md".to_string(),
                "apps/service/AGENTS.md".to_string()
            ]
        );
        assert!(docs.iter().all(|doc| doc.path != ".codex/instructions.md"));
    }

    #[test]
    fn discover_project_documents_truncates_to_32kb_with_marker() {
        let tmp = tempdir().expect("temp dir should be created");
        let root = tmp.path();
        let nested = root.join("workspace");
        fs::create_dir_all(&nested).expect("nested dir should be created");
        fs::create_dir_all(root.join(".git")).expect(".git marker dir should be created");

        let oversized = "A".repeat(40 * 1024);
        write_test_file(&root.join("AGENTS.md"), &oversized);

        let profile = StaticProviderProfile {
            id: "openai".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "base".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        };

        let docs = discover_project_documents(&nested, &profile);
        assert_eq!(docs.len(), 1);
        assert!(docs[0].content.contains(PROJECT_DOC_TRUNCATION_MARKER));
        assert!(docs[0].content.len() <= (32 * 1024) + PROJECT_DOC_TRUNCATION_MARKER.len() + 1);
    }

    fn build_tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
            raw_arguments: None,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn subagent_tools_spawn_and_wait_flow_returns_deterministic_result() {
        let (client, _) = build_test_client(vec![text_response("child-resp-1", "child complete")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        let spawn = session
            .execute_subagent_tool_call(build_tool_call(
                "call-1",
                "spawn_agent",
                serde_json::json!({ "task": "do child task" }),
            ))
            .await
            .expect("spawn should execute");
        assert!(!spawn.is_error);
        let spawn_payload: Value = serde_json::from_str(
            spawn
                .content
                .as_str()
                .expect("spawn payload should be string JSON"),
        )
        .expect("spawn payload should parse");
        let agent_id = spawn_payload
            .get("agent_id")
            .and_then(Value::as_str)
            .expect("agent_id must exist");
        assert_eq!(
            spawn_payload.get("status").and_then(Value::as_str),
            Some("running")
        );

        let wait = session
            .execute_subagent_tool_call(build_tool_call(
                "call-2",
                "wait",
                serde_json::json!({ "agent_id": agent_id }),
            ))
            .await
            .expect("wait should execute");
        assert!(!wait.is_error);
        let wait_payload: Value = serde_json::from_str(
            wait.content
                .as_str()
                .expect("wait payload should be string JSON"),
        )
        .expect("wait payload should parse");
        assert_eq!(
            wait_payload.get("agent_id").and_then(Value::as_str),
            Some(agent_id)
        );
        assert_eq!(
            wait_payload.get("status").and_then(Value::as_str),
            Some("completed")
        );
        assert_eq!(
            wait_payload.get("success").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_agent_honors_model_override_for_child_requests() {
        let (client, requests) = build_test_client(vec![text_response("child-resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(build_openai_tool_registry()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        let spawn = session
            .execute_subagent_tool_call(build_tool_call(
                "call-1",
                "spawn_agent",
                serde_json::json!({ "task": "do child task", "model": "override-model" }),
            ))
            .await
            .expect("spawn should execute");
        assert!(!spawn.is_error);
        let spawn_payload: Value = serde_json::from_str(
            spawn
                .content
                .as_str()
                .expect("spawn payload should be string JSON"),
        )
        .expect("spawn payload should parse");
        let agent_id = spawn_payload
            .get("agent_id")
            .and_then(Value::as_str)
            .expect("agent_id must exist");

        let wait = session
            .execute_subagent_tool_call(build_tool_call(
                "call-2",
                "wait",
                serde_json::json!({ "agent_id": agent_id }),
            ))
            .await
            .expect("wait should execute");
        assert!(!wait.is_error);

        let seen_requests = requests.lock().expect("requests mutex").clone();
        assert_eq!(seen_requests.len(), 1);
        assert_eq!(seen_requests[0].model, "override-model");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_agent_honors_working_dir_scope_for_child_tools() {
        let temp = tempdir().expect("temp dir should exist");
        let scoped_dir = temp.path().join("scoped");
        fs::create_dir_all(&scoped_dir).expect("scoped dir should exist");
        fs::write(scoped_dir.join("only.txt"), "scoped-data\n").expect("seed file should write");

        let (client, _requests) = build_test_client(vec![
            tool_call_response(
                "child-resp-1",
                "call-read",
                "read_file",
                serde_json::json!({ "file_path": "only.txt", "offset": 1, "limit": 10 }),
            ),
            text_response("child-resp-2", "done"),
        ]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(build_openai_tool_registry()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(temp.path()));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        let spawn = session
            .execute_subagent_tool_call(build_tool_call(
                "call-1",
                "spawn_agent",
                serde_json::json!({ "task": "read file", "working_dir": "scoped" }),
            ))
            .await
            .expect("spawn should execute");
        assert!(!spawn.is_error);
        let spawn_payload: Value = serde_json::from_str(
            spawn
                .content
                .as_str()
                .expect("spawn payload should be string JSON"),
        )
        .expect("spawn payload should parse");
        let agent_id = spawn_payload
            .get("agent_id")
            .and_then(Value::as_str)
            .expect("agent_id must exist");

        let wait = session
            .execute_subagent_tool_call(build_tool_call(
                "call-2",
                "wait",
                serde_json::json!({ "agent_id": agent_id }),
            ))
            .await
            .expect("wait should execute");
        assert!(!wait.is_error);

        let record = session
            .subagent_records
            .get(agent_id)
            .expect("subagent record should exist");
        let child = record
            .session
            .as_ref()
            .expect("child session should be available");
        let read_result = child.history().iter().find_map(|turn| {
            if let Turn::ToolResults(results) = turn {
                results
                    .results
                    .iter()
                    .find(|result| result.tool_call_id == "call-read")
                    .cloned()
            } else {
                None
            }
        });
        let read_result = read_result.expect("read_file result should be present");
        assert!(!read_result.is_error);
        assert!(
            read_result
                .content
                .as_str()
                .unwrap_or_default()
                .contains("scoped-data")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_agent_rejects_when_depth_limit_reached() {
        let (client, _) = build_test_client(vec![]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut config = SessionConfig::default();
        config.max_subagent_depth = 0;
        let mut session = Session::new(profile, env, client, config).expect("new session");

        let result = session
            .execute_subagent_tool_call(build_tool_call(
                "call-1",
                "spawn_agent",
                serde_json::json!({ "task": "blocked" }),
            ))
            .await
            .expect("tool execution should not panic");

        assert!(result.is_error);
        assert!(
            result
                .content
                .as_str()
                .unwrap_or_default()
                .contains("max_subagent_depth")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_closes_all_subagents_and_updates_status() {
        let (client, _) = build_test_client(vec![text_response("child-resp-1", "done")]);
        let profile = Arc::new(StaticProviderProfile {
            id: "test".to_string(),
            model: "gpt-5.2-codex".to_string(),
            base_system_prompt: "system".to_string(),
            tool_registry: Arc::new(ToolRegistry::default()),
            provider_options: None,
            capabilities: ProviderCapabilities::default(),
        });
        let env = Arc::new(LocalExecutionEnvironment::new(PathBuf::from(".")));
        let mut session =
            Session::new(profile, env, client, SessionConfig::default()).expect("new session");

        let spawn = session
            .execute_subagent_tool_call(build_tool_call(
                "call-1",
                "spawn_agent",
                serde_json::json!({ "task": "run child" }),
            ))
            .await
            .expect("spawn should execute");
        let spawn_payload: Value =
            serde_json::from_str(spawn.content.as_str().expect("spawn content")).expect("json");
        let agent_id = spawn_payload
            .get("agent_id")
            .and_then(Value::as_str)
            .expect("agent id");
        assert!(session.subagents.contains_key(agent_id));

        session.close().expect("close should succeed");
        assert_eq!(session.state(), &SessionState::Closed);
        assert!(matches!(
            session.subagents.get(agent_id).map(|h| &h.status),
            Some(SubAgentStatus::Failed)
        ));
    }
}
