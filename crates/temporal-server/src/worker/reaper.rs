//! Deployment-level promise/source repair loop.
//!
//! Workflow-local machinery handles the normal path: terminal runs notify
//! holder workflows, and holder-side promise cancellation flushes source
//! cancellation. This reaper is the backstop for the cases that no single
//! workflow can repair by itself: missed signals, terminated workflows, or
//! promise/source state that is only visible by scanning session logs.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use api_projection::{MAX_EVENT_PAGE_LIMIT, read_all_session_entries, replay_core_agent_state};
use async_trait::async_trait;
use engine::{
    CoreAgentAction, CoreAgentCommand, CoreAgentDrive, CoreAgentState, CoreAgentStatus, Promise,
    PromiseId, PromiseResolution, PromiseScope, PromiseSource, SessionId,
    storage::{
        AppendSessionEvents, ListSessions, SessionListCursor, SessionRecord, SessionStore,
        SessionStoreError,
    },
};
use temporal_workflow::{AgentAdmission, AgentSessionWorkflow, compose_workflow_id};
use temporalio_client::{
    Client, WorkflowDescribeOptions, WorkflowSignalOptions, errors::WorkflowInteractionError,
};
use temporalio_common::protos::temporal::api::enums::v1::WorkflowExecutionStatus;
use thiserror::Error;
use uuid::Uuid;

use crate::config::DeploymentStores;

const DEFAULT_REAPER_INTERVAL: Duration = Duration::from_secs(5 * 60);
const SESSION_PAGE_LIMIT: usize = 256;

#[derive(Clone)]
pub struct PromiseReaper {
    client: Client,
    stores: DeploymentStores,
    interval: Duration,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReaperStats {
    pub universes_scanned: usize,
    pub sessions_scanned: usize,
    pub promises_examined: usize,
    pub holder_repairs_signalled: usize,
    pub holder_repairs_appended: usize,
    pub stale_active_projections: usize,
    pub workflow_status_errors: usize,
    pub conflicts: usize,
    pub errors: usize,
}

impl ReaperStats {
    fn merge(&mut self, other: Self) {
        self.universes_scanned += other.universes_scanned;
        self.sessions_scanned += other.sessions_scanned;
        self.promises_examined += other.promises_examined;
        self.holder_repairs_signalled += other.holder_repairs_signalled;
        self.holder_repairs_appended += other.holder_repairs_appended;
        self.stale_active_projections += other.stale_active_projections;
        self.workflow_status_errors += other.workflow_status_errors;
        self.conflicts += other.conflicts;
        self.errors += other.errors;
    }

    fn repaired_anything(&self) -> bool {
        self.holder_repairs_signalled > 0 || self.holder_repairs_appended > 0
    }
}

impl PromiseReaper {
    pub fn new(client: Client, stores: DeploymentStores) -> Self {
        Self {
            client,
            stores,
            interval: DEFAULT_REAPER_INTERVAL,
        }
    }

    pub async fn run_forever(self) {
        loop {
            match self.run_once().await {
                Ok(stats)
                    if stats.repaired_anything()
                        || stats.stale_active_projections > 0
                        || stats.workflow_status_errors > 0
                        || stats.errors > 0
                        || stats.conflicts > 0 =>
                {
                    tracing::info!(
                        target: "temporal_server",
                        universes_scanned = stats.universes_scanned,
                        sessions_scanned = stats.sessions_scanned,
                        promises_examined = stats.promises_examined,
                        holder_repairs_signalled = stats.holder_repairs_signalled,
                        holder_repairs_appended = stats.holder_repairs_appended,
                        stale_active_projections = stats.stale_active_projections,
                        workflow_status_errors = stats.workflow_status_errors,
                        conflicts = stats.conflicts,
                        errors = stats.errors,
                        "promise reaper pass complete"
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        target: "temporal_server",
                        %error,
                        "promise reaper pass failed"
                    );
                }
            }
            tokio::time::sleep(self.interval).await;
        }
    }

    pub async fn run_once(&self) -> anyhow::Result<ReaperStats> {
        let workflows: Arc<dyn WorkflowRepairClient> = Arc::new(TemporalWorkflowRepairClient {
            client: self.client.clone(),
        });
        let universes = store_pg::list_universes(self.stores.pool()).await?;
        let mut stats = ReaperStats::default();
        for (universe_id, _) in universes {
            let store = self.stores.store_for(universe_id);
            let sessions: Arc<dyn SessionStore> = store.clone();
            let append_store: Arc<dyn SessionStore> = store;
            let universe_stats = reap_universe_once(
                universe_id,
                sessions,
                append_store,
                workflows.clone(),
                now_ms(),
            )
            .await?;
            stats.merge(universe_stats);
        }
        Ok(stats)
    }
}

#[derive(Clone)]
struct LoadedSessionSnapshot {
    record: SessionRecord,
    state: CoreAgentState,
}

#[derive(Default)]
struct ReaperPlan {
    holder_commands: BTreeMap<SessionId, Vec<CoreAgentCommand>>,
}

pub(super) async fn reap_universe_once(
    universe_id: Uuid,
    sessions: Arc<dyn SessionStore>,
    append_store: Arc<dyn SessionStore>,
    workflows: Arc<dyn WorkflowRepairClient>,
    now_ms: u64,
) -> anyhow::Result<ReaperStats> {
    let snapshots = load_session_snapshots(sessions.as_ref()).await?;
    let mut workflow_status_cache = BTreeMap::<SessionId, SessionWorkflowStatus>::new();
    let mut stats = ReaperStats {
        universes_scanned: 1,
        sessions_scanned: snapshots.len(),
        ..ReaperStats::default()
    };
    observe_active_projection_statuses(
        universe_id,
        &snapshots,
        workflows.as_ref(),
        &mut workflow_status_cache,
        &mut stats,
    )
    .await;
    let plan = plan_repair(&snapshots, now_ms, &mut stats);

    apply_holder_repairs(
        universe_id,
        append_store.clone(),
        workflows.as_ref(),
        &snapshots,
        plan.holder_commands,
        now_ms,
        &mut stats,
    )
    .await;
    Ok(stats)
}

fn plan_repair(
    snapshots: &BTreeMap<SessionId, LoadedSessionSnapshot>,
    now_ms: u64,
    stats: &mut ReaperStats,
) -> ReaperPlan {
    let mut plan = ReaperPlan::default();
    for (holder_session_id, holder) in snapshots {
        for promise in holder.state.promises.pending() {
            stats.promises_examined += 1;
            if !promise_owner_live(&holder.state, promise) {
                plan_holder_resolution(
                    &mut plan,
                    holder_session_id,
                    promise.promise_id.clone(),
                    PromiseResolution::Cancelled,
                );
                continue;
            }

            if let Some(resolution) = promise_source_resolution(&promise.source, now_ms) {
                plan_holder_resolution(
                    &mut plan,
                    holder_session_id,
                    promise.promise_id.clone(),
                    resolution,
                );
            }
        }
    }
    plan
}

fn plan_holder_resolution(
    plan: &mut ReaperPlan,
    holder_session_id: &SessionId,
    promise_id: PromiseId,
    resolution: PromiseResolution,
) {
    plan.holder_commands
        .entry(holder_session_id.clone())
        .or_default()
        .push(CoreAgentCommand::ResolvePromise {
            promise_id,
            resolution,
        });
}

fn promise_source_resolution(source: &PromiseSource, now_ms: u64) -> Option<PromiseResolution> {
    match source {
        PromiseSource::Timer { fire_at_ms } => {
            (*fire_at_ms <= now_ms).then_some(PromiseResolution::Resolved { payload_ref: None })
        }
        // Only the exact stored producer may resolve a workflow-tool
        // promise; the reaper cannot poll or repair it. Terminal delivery
        // failure and session close remain the unresolvable-promise
        // backstops.
        PromiseSource::Workflow { .. } => None,
    }
}

fn promise_owner_live(state: &CoreAgentState, promise: &Promise) -> bool {
    match promise.scope {
        PromiseScope::Run { run_id } => state
            .runs
            .active
            .as_ref()
            .is_some_and(|run| run.run_id == run_id),
        PromiseScope::Session => state.lifecycle.status != CoreAgentStatus::Closed,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SessionWorkflowStatus {
    Running,
    Terminal(WorkflowExecutionStatus),
    NotFound,
    Unavailable(String),
}

async fn workflow_status_cached(
    workflows: &dyn WorkflowRepairClient,
    cache: &mut BTreeMap<SessionId, SessionWorkflowStatus>,
    universe_id: Uuid,
    session_id: &SessionId,
) -> SessionWorkflowStatus {
    if let Some(status) = cache.get(session_id) {
        return status.clone();
    }
    let status = workflows.workflow_status(universe_id, session_id).await;
    cache.insert(session_id.clone(), status.clone());
    status
}

async fn observe_active_projection_statuses(
    universe_id: Uuid,
    snapshots: &BTreeMap<SessionId, LoadedSessionSnapshot>,
    workflows: &dyn WorkflowRepairClient,
    cache: &mut BTreeMap<SessionId, SessionWorkflowStatus>,
    stats: &mut ReaperStats,
) {
    for (session_id, snapshot) in snapshots {
        let active_run_id = snapshot
            .state
            .runs
            .active
            .as_ref()
            .map(|run| run.run_id)
            .or_else(|| snapshot.state.runs.queued.first().map(|run| run.run_id));
        let Some(active_run_id) = active_run_id else {
            continue;
        };

        let workflow_id = compose_workflow_id(universe_id, session_id);
        let session_head_seq = snapshot
            .record
            .head
            .as_ref()
            .map(|head| head.seq.to_string())
            .unwrap_or_default();
        match workflow_status_cached(workflows, cache, universe_id, session_id).await {
            SessionWorkflowStatus::Running => {}
            SessionWorkflowStatus::Terminal(status) => {
                stats.stale_active_projections += 1;
                tracing::error!(
                    target: "temporal_server",
                    event = "stale_active_projection",
                    %universe_id,
                    %session_id,
                    %workflow_id,
                    lightspeed_run_id = active_run_id.as_u64(),
                    session_head_seq,
                    temporal_status = status.as_str_name(),
                    "session projection is active but its workflow is terminal"
                );
            }
            SessionWorkflowStatus::NotFound => {
                stats.stale_active_projections += 1;
                tracing::error!(
                    target: "temporal_server",
                    event = "stale_active_projection",
                    %universe_id,
                    %session_id,
                    %workflow_id,
                    lightspeed_run_id = active_run_id.as_u64(),
                    session_head_seq,
                    temporal_status = "not_found",
                    "session projection is active but its workflow was not found"
                );
            }
            SessionWorkflowStatus::Unavailable(error) => {
                stats.workflow_status_errors += 1;
                tracing::warn!(
                    target: "temporal_server",
                    event = "session_workflow_status_unavailable",
                    %universe_id,
                    %session_id,
                    %workflow_id,
                    lightspeed_run_id = active_run_id.as_u64(),
                    session_head_seq,
                    %error,
                    "could not verify active session projection against Temporal"
                );
            }
        }
    }
}

async fn apply_holder_repairs(
    universe_id: Uuid,
    store: Arc<dyn SessionStore>,
    workflows: &dyn WorkflowRepairClient,
    snapshots: &BTreeMap<SessionId, LoadedSessionSnapshot>,
    holder_commands: BTreeMap<SessionId, Vec<CoreAgentCommand>>,
    now_ms: u64,
    stats: &mut ReaperStats,
) {
    for (session_id, commands) in holder_commands {
        match workflows
            .signal_admissions(universe_id, &session_id, admissions(commands.clone()))
            .await
        {
            Ok(()) => {
                stats.holder_repairs_signalled += commands.len();
                continue;
            }
            Err(WorkflowSignalFailure::NotFound) => {}
            Err(WorkflowSignalFailure::Other(error)) => {
                stats.errors += 1;
                tracing::warn!(
                    target: "temporal_server",
                    %universe_id,
                    %session_id,
                    %error,
                    "promise reaper failed to signal holder repair"
                );
                continue;
            }
        }
        let Some(snapshot) = snapshots.get(&session_id) else {
            continue;
        };
        match append_commands_direct(store.as_ref(), &session_id, snapshot, commands, now_ms).await
        {
            Ok(appended) => stats.holder_repairs_appended += appended,
            Err(DirectAppendError::Conflict) => stats.conflicts += 1,
            Err(DirectAppendError::Other(error)) => {
                stats.errors += 1;
                tracing::warn!(
                    target: "temporal_server",
                    %universe_id,
                    %session_id,
                    %error,
                    "promise reaper failed to append holder repair"
                );
            }
        }
    }
}

fn admissions(commands: Vec<CoreAgentCommand>) -> Vec<AgentAdmission> {
    commands
        .into_iter()
        .map(|command| AgentAdmission {
            command,
            correlation_token: None,
        })
        .collect()
}

#[derive(Debug, Error)]
enum DirectAppendError {
    #[error("expected head conflict")]
    Conflict,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

async fn append_commands_direct(
    store: &dyn SessionStore,
    session_id: &SessionId,
    snapshot: &LoadedSessionSnapshot,
    commands: Vec<CoreAgentCommand>,
    now_ms: u64,
) -> Result<usize, DirectAppendError> {
    let mut drive = CoreAgentDrive::from_replayed(
        session_id.clone(),
        snapshot.state.clone(),
        snapshot.record.head.clone(),
    );
    let mut appended_count = 0usize;
    for command in commands {
        let action = drive
            .admit_command(command, now_ms)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        match action {
            CoreAgentAction::AppendEvents {
                expected_head,
                events,
            } => {
                let result = store
                    .append(AppendSessionEvents {
                        session_id: session_id.clone(),
                        expected_head,
                        events,
                    })
                    .await
                    .map_err(|error| match error {
                        SessionStoreError::ExpectedHeadMismatch { .. } => {
                            DirectAppendError::Conflict
                        }
                        other => DirectAppendError::Other(anyhow::anyhow!("{other}")),
                    })?;
                if !result.entries.is_empty() {
                    appended_count += 1;
                }
                drive
                    .resume_appended(result.entries)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
            }
            CoreAgentAction::Idle | CoreAgentAction::Closed => {}
            other => {
                return Err(DirectAppendError::Other(anyhow::anyhow!(
                    "direct repair command produced unexpected action: {other:?}"
                )));
            }
        }
    }
    Ok(appended_count)
}

async fn load_session_snapshots(
    sessions: &dyn SessionStore,
) -> anyhow::Result<BTreeMap<SessionId, LoadedSessionSnapshot>> {
    let mut cursor: Option<SessionListCursor> = None;
    let mut snapshots = BTreeMap::new();
    loop {
        let page = sessions
            .list_sessions(ListSessions {
                cursor,
                limit: SESSION_PAGE_LIMIT,
                root_session_id: None,
                parent_session_id: None,
            })
            .await?;
        for record in page.sessions {
            let entries = read_all_session_entries(
                sessions,
                &record.session_id,
                MAX_EVENT_PAGE_LIMIT as usize,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
            let state =
                replay_core_agent_state(&entries).map_err(|error| anyhow::anyhow!("{error}"))?;
            snapshots.insert(
                record.session_id.clone(),
                LoadedSessionSnapshot { record, state },
            );
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(snapshots);
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum WorkflowSignalFailure {
    #[error("workflow not found")]
    NotFound,
    #[error("{0}")]
    Other(String),
}

#[async_trait]
pub(super) trait WorkflowRepairClient: Send + Sync {
    async fn signal_admissions(
        &self,
        universe_id: Uuid,
        session_id: &SessionId,
        admissions: Vec<AgentAdmission>,
    ) -> Result<(), WorkflowSignalFailure>;

    async fn workflow_status(
        &self,
        universe_id: Uuid,
        session_id: &SessionId,
    ) -> SessionWorkflowStatus;
}

struct TemporalWorkflowRepairClient {
    client: Client,
}

#[async_trait]
impl WorkflowRepairClient for TemporalWorkflowRepairClient {
    async fn signal_admissions(
        &self,
        universe_id: Uuid,
        session_id: &SessionId,
        admissions: Vec<AgentAdmission>,
    ) -> Result<(), WorkflowSignalFailure> {
        let workflow_id = compose_workflow_id(universe_id, session_id);
        match self
            .client
            .get_workflow_handle::<AgentSessionWorkflow>(workflow_id)
            .signal(
                AgentSessionWorkflow::submit_admissions,
                admissions,
                WorkflowSignalOptions::default(),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(WorkflowInteractionError::NotFound(_)) => Err(WorkflowSignalFailure::NotFound),
            Err(error) => Err(WorkflowSignalFailure::Other(error.to_string())),
        }
    }

    async fn workflow_status(
        &self,
        universe_id: Uuid,
        session_id: &SessionId,
    ) -> SessionWorkflowStatus {
        let workflow_id = compose_workflow_id(universe_id, session_id);
        match self
            .client
            .get_workflow_handle::<AgentSessionWorkflow>(workflow_id)
            .describe(WorkflowDescribeOptions::default())
            .await
        {
            Ok(description) if description.status() == WorkflowExecutionStatus::Running => {
                SessionWorkflowStatus::Running
            }
            Ok(description) => SessionWorkflowStatus::Terminal(description.status()),
            Err(WorkflowInteractionError::NotFound(_)) => SessionWorkflowStatus::NotFound,
            Err(error) => SessionWorkflowStatus::Unavailable(error.to_string()),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Mutex;

    use engine::{ActiveRun, ModelSelection, ProviderApiKind, RunId, RunSource, RunStatus};
    use temporal_workflow::{DEFAULT_MODEL, default_run_config, default_session_config};

    use super::*;

    #[derive(Default)]
    struct FakeWorkflows {
        running: BTreeSet<SessionId>,
        statuses: BTreeMap<SessionId, SessionWorkflowStatus>,
        signals: Mutex<Vec<(SessionId, Vec<AgentAdmission>)>>,
    }

    #[async_trait]
    impl WorkflowRepairClient for FakeWorkflows {
        async fn signal_admissions(
            &self,
            _universe_id: Uuid,
            session_id: &SessionId,
            admissions: Vec<AgentAdmission>,
        ) -> Result<(), WorkflowSignalFailure> {
            if !self.running.contains(session_id) {
                return Err(WorkflowSignalFailure::NotFound);
            }
            self.signals
                .lock()
                .expect("signals lock")
                .push((session_id.clone(), admissions));
            Ok(())
        }

        async fn workflow_status(
            &self,
            _universe_id: Uuid,
            session_id: &SessionId,
        ) -> SessionWorkflowStatus {
            if let Some(status) = self.statuses.get(session_id) {
                status.clone()
            } else if self.running.contains(session_id) {
                SessionWorkflowStatus::Running
            } else {
                SessionWorkflowStatus::NotFound
            }
        }
    }

    #[tokio::test]
    async fn terminal_workflow_with_active_run_is_observed_as_stale() {
        let universe_id = Uuid::new_v4();
        let session_id = SessionId::new("stale");
        let mut state = open_state();
        state.runs.active = Some(active_run(RunId::new(7)));
        let snapshots = snapshots([(session_id.clone(), state)]);
        let workflows = FakeWorkflows {
            statuses: BTreeMap::from([(
                session_id,
                SessionWorkflowStatus::Terminal(WorkflowExecutionStatus::Failed),
            )]),
            ..FakeWorkflows::default()
        };
        let mut cache = BTreeMap::new();
        let mut stats = ReaperStats::default();

        observe_active_projection_statuses(
            universe_id,
            &snapshots,
            &workflows,
            &mut cache,
            &mut stats,
        )
        .await;

        assert_eq!(stats.stale_active_projections, 1);
        assert_eq!(stats.workflow_status_errors, 0);
    }

    fn open_state() -> CoreAgentState {
        let mut state = CoreAgentState::new();
        state.lifecycle.status = CoreAgentStatus::Open;
        state.lifecycle.config = Some(default_session_config(test_model()));
        state
    }

    fn test_model() -> ModelSelection {
        ModelSelection {
            api_kind: ProviderApiKind::OpenAiResponses,
            provider_id: "openai".to_owned(),
            model: DEFAULT_MODEL.to_owned(),
        }
    }

    fn active_run(run_id: RunId) -> ActiveRun {
        ActiveRun {
            run_id,
            status: RunStatus::Active,
            submission_id: None,
            source: RunSource::Input { input: Vec::new() },
            input_entry_ids: Vec::new(),
            input_consumed_by_turn_id: None,
            run_config: default_run_config(),
            config_revision: 0,
            steering: Vec::new(),
            turns: BTreeMap::new(),
            active_turn_id: None,
            active_tool_batch_id: None,
            approvals: Default::default(),
            parked_tool_batch: None,
            tool_batches: BTreeMap::new(),
            completed_tool_batches: BTreeMap::new(),
            output_ref: None,
            failure: None,
            notify_on_terminal: Vec::new(),
        }
    }

    fn snapshots(
        states: impl IntoIterator<Item = (SessionId, CoreAgentState)>,
    ) -> BTreeMap<SessionId, LoadedSessionSnapshot> {
        states
            .into_iter()
            .map(|(session_id, state)| {
                (
                    session_id.clone(),
                    LoadedSessionSnapshot {
                        record: SessionRecord {
                            session_id,
                            display_name: None,
                            lifecycle_status: engine::storage::SessionLifecycleStatus::New,
                            closed_at_seq: None,
                            managed: false,
                            head: None,
                            source_session_id: None,
                            source_seq: None,
                            origin: None,
                            created_at_ms: 0,
                            updated_at_ms: 0,
                        },
                        state,
                    },
                )
            })
            .collect()
    }
}
