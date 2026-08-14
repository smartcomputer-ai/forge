//! Hosted Fleet subagent control-plane service.

use std::sync::Arc;

use api::{
    AgentApiError, AgentApiService, AgentProfile, AgentProfileSummary, EventCursor, InputItem,
    MediaKind, ProfileId, ProfileListParams, ProfileReadParams, ProfileSource,
    RunStatus as ApiRunStatus, SessionEventsReadParams, SessionEventsReadResponse,
    SessionReadParams, SessionView,
};
use async_trait::async_trait;
use engine::{
    BlobRef, CoreAgentAction, CoreAgentCommand, CoreAgentDrive, CoreAgentIoError, EventSeq,
    PromiseId, PromiseSource, RunId, RunTerminalNotifyIntent, SessionId, SubmissionId, ToolBatchId,
    ToolBatchOutcome, ToolCallId, ToolCallStatus, ToolInvocationBatchResult, ToolInvocationRequest,
    ToolInvocationResult, TurnId, core_agent_clone_opening_events, promise_create_effect,
    storage::{
        AppendSessionEvents, BlobStore, BlobStoreError, CreateClonedSession, CreateForkedSession,
        ListSessionLinks, SessionLinkDirection, SessionRecord, SessionStore, SessionStoreError,
        UpsertSessionLink,
    },
};
use serde::Serialize;
use serde_json::{Value, json};
use tools::{
    concurrency::{
        AWAIT_TOOL_NAME, AwaitArgs, AwaitModeArg, CANCEL_TOOL_NAME, CancelArgs, DETACH_TOOL_NAME,
        DetachArgs, cancel_promises_from_runtime, cancel_promises_model_visible_text,
        detach_promises_from_runtime, detach_promises_model_visible_text,
    },
    fleet::{
        AGENT_LIST_TOOL_NAME, AGENT_READ_TOOL_NAME, AGENT_REQUEST_TOOL_NAME, AGENT_SEND_TOOL_NAME,
        AGENT_SPAWN_TOOL_NAME, AgentLineageView, AgentLinkView, AgentListArgs, AgentListDirection,
        AgentListItem, AgentListOutput, AgentReadArgs, AgentReadOutput, AgentRequestArgs,
        AgentRequestOutput, AgentSendArgs, AgentSendInputItem, AgentSendMediaKind, AgentSendOutput,
        AgentSendTarget, AgentSpawnArgs, AgentSpawnBase, AgentSpawnFork, AgentSpawnOutput,
        EnvironmentPolicy, PROFILE_LIST_TOOL_NAME, PROFILE_READ_TOOL_NAME, ProfileListArgs,
        ProfileListOutput, ProfileReadArgs, ProfileReadOutput, VfsPolicy,
    },
};
use vfs::{CreateVfsWorkspaceRecord, VfsCatalogError, VfsPath, VfsWorkspaceId, VfsWorkspaceStore};

use crate::gateway::GatewayAgentApi;

pub const FLEET_CHILD_RELATIONSHIP: &str = "fleet_child";
const DEFAULT_AGENT_LIST_LIMIT: usize = 20;
const MAX_AGENT_LIST_LIMIT: usize = 100;
const DEFAULT_RECENT_EVENT_LIMIT: u32 = 20;
const DEFAULT_RECENT_TRANSCRIPT_EVENT_LIMIT: u32 = 20;
const MAX_RECENT_EVENT_LIMIT: u32 = 100;
const MAX_DIRECT_LINKS: usize = 100;

#[derive(Clone)]
pub struct FleetService {
    sessions: Arc<dyn SessionStore>,
    runtime: Arc<dyn FleetChildRuntime>,
    workspace_store: Option<Arc<dyn VfsWorkspaceStore>>,
}

impl FleetService {
    pub fn new(sessions: Arc<dyn SessionStore>, runtime: Arc<dyn FleetChildRuntime>) -> Self {
        Self {
            sessions,
            runtime,
            workspace_store: None,
        }
    }

    pub fn with_vfs_stores(mut self, workspace_store: Arc<dyn VfsWorkspaceStore>) -> Self {
        self.workspace_store = Some(workspace_store);
        self
    }

    pub async fn spawn(
        &self,
        context: FleetInvocationContext,
        args: AgentSpawnArgs,
    ) -> Result<SpawnResult, AgentApiError> {
        validate_spawn_args(&args)?;
        let fleet_config = fleet_policy(&context)?;
        validate_spawn_policy(fleet_config, &args)?;
        let child_id_was_derived = args.child_session_id.is_none();
        let child_session_id = match args.child_session_id.as_deref() {
            Some(session_id) => parse_session_id(session_id, "child_session_id")?,
            None => derived_child_session_id(&context),
        };
        let spawn_request_id = spawn_request_id(&context);
        let child_run_submission_id = child_run_submission_id(&context);

        let (outcome, source_session_id, source_seq) = if let Some(profile) = args.base.profile() {
            let existed = self
                .sessions
                .load_session(&child_session_id)
                .await
                .map_err(map_session_store_error)?
                .is_some();
            self.runtime
                .start_session(
                    &child_session_id,
                    args.lifecycle.close_on_terminal,
                    Some(profile.clone()),
                )
                .await?;
            if !existed {
                (ChildCreateOutcome::Created, None, None)
            } else {
                (
                    ChildCreateOutcome::Reused {
                        matching_spawn_link: false,
                    },
                    None,
                    None,
                )
            }
        } else {
            let source_session_id = self.resolve_base_source(&context, &args.base)?;
            let source_record = self.load_session_required(&source_session_id).await?;
            if source_record.managed {
                return Err(AgentApiError::rejected(format!(
                    "lifecycle-managed session cannot be cloned or forked: {source_session_id}"
                )));
            }
            let source_seq = if let Some(fork) = args.base.fork() {
                Some(match fork {
                    AgentSpawnFork::Safe => self
                        .sessions
                        .safe_fork_seq(&source_session_id)
                        .await
                        .map_err(map_session_store_error)?,
                    AgentSpawnFork::AtSeq { seq } => EventSeq::new(*seq),
                })
            } else {
                None
            };

            let outcome = self
                .create_or_reuse_child(
                    &context,
                    &source_record,
                    &child_session_id,
                    source_seq,
                    &spawn_request_id,
                    child_id_was_derived,
                )
                .await?;
            (outcome, Some(source_session_id), source_seq)
        };
        let skip_pre_run_setup = outcome.has_matching_spawn_link();
        if args.base.profile().is_none() && !skip_pre_run_setup {
            self.apply_resource_policies(
                source_session_id
                    .as_ref()
                    .expect("non-profile spawn has a source session"),
                &child_session_id,
                context.observed_at_ms,
                &args,
            )
            .await?;
        }

        if args.base.profile().is_none() {
            self.runtime
                .start_session(&child_session_id, args.lifecycle.close_on_terminal, None)
                .await?;
        }
        self.upsert_spawn_link(
            &context,
            source_session_id.as_ref(),
            &child_session_id,
            source_seq,
            &spawn_request_id,
            &args,
        )
        .await?;
        // Mint the promise id from stable inputs available before the run
        // starts, so a spawn retry is deterministic. The parent holds this
        // promise; the child run carries a notify-intent back to the parent
        // workflow, keyed by the same id.
        let promise_id = spawn_promise_id(&context, &child_session_id);
        let holder_workflow_id = self
            .runtime
            .holder_workflow_id(&context.parent_session_id)
            .await?;
        let notify_intents = vec![RunTerminalNotifyIntent {
            holder_workflow_id,
            token: promise_id.clone(),
        }];
        let child_run_id = if args.lifecycle.run_immediately {
            Some(
                self.runtime
                    .start_run(
                        &child_session_id,
                        vec![InputItem::Text {
                            text: spawn_run_text(&context, &args),
                        }],
                        child_run_submission_id,
                        notify_intents,
                    )
                    .await?,
            )
        } else {
            None
        };

        // A promise is created only when a run actually started — its
        // resolution is that run's terminal state.
        let promise = child_run_id.as_ref().map(|run_id| {
            let target_run_id = parse_api_run_id_u64(run_id);
            let effect = promise_create_effect(
                &PromiseId::new(&promise_id),
                &PromiseSource::Run {
                    target_session_id: child_session_id.as_str().to_owned(),
                    target_run_id,
                },
                None,
            );
            (promise_id.clone(), effect)
        });

        Ok(SpawnResult {
            output: AgentSpawnOutput {
                child_session_id: child_session_id.as_str().to_owned(),
                child_run_id,
                status: if matches!(outcome, ChildCreateOutcome::Created) {
                    "created".to_owned()
                } else {
                    "reused".to_owned()
                },
                promise: promise.as_ref().map(|(id, _)| id.clone()),
            },
            promise_effect: promise.map(|(_, effect)| effect),
        })
    }

    pub async fn list_profiles(
        &self,
        context: &FleetInvocationContext,
        _args: ProfileListArgs,
    ) -> Result<ProfileListOutput, AgentApiError> {
        let fleet_config = fleet_policy(context)?;
        Ok(ProfileListOutput {
            profiles: self
                .runtime
                .list_profiles()
                .await?
                .into_iter()
                .filter(|profile| {
                    fleet_config
                        .profiles
                        .named_profile_allowed(profile.profile_id.as_str())
                })
                .collect(),
        })
    }

    pub async fn read_profile(
        &self,
        context: &FleetInvocationContext,
        args: ProfileReadArgs,
    ) -> Result<ProfileReadOutput, AgentApiError> {
        let fleet_config = fleet_policy(context)?;
        let profile_id = ProfileId::try_new(args.profile_id).map_err(|error| {
            AgentApiError::invalid_request(format!("invalid profile_id: {error}"))
        })?;
        validate_named_profile_allowed(fleet_config, &profile_id)?;
        Ok(ProfileReadOutput {
            profile: self.runtime.read_profile(profile_id).await?,
        })
    }

    pub async fn send(
        &self,
        context: FleetInvocationContext,
        args: AgentSendArgs,
    ) -> Result<SendResult, AgentApiError> {
        validate_send_args(&args)?;
        let target_session_id = self
            .resolve_send_target(&context, &args.to)
            .await?
            .ok_or_else(|| AgentApiError::rejected("calling session has no Fleet parent"))?;
        if !self
            .has_session_link_edge(&context.parent_session_id, &target_session_id)
            .await?
        {
            return Err(AgentApiError::rejected(format!(
                "session {target_session_id} is not directly linked to {}",
                context.parent_session_id
            )));
        }
        let submission_id = send_submission_id(&context, &target_session_id);
        let input = send_run_input(&context, &args)?;
        self.runtime
            .deliver_message(&target_session_id, input, submission_id.clone())
            .await?;
        Ok(SendResult {
            output: AgentSendOutput {
                target_session_id: target_session_id.as_str().to_owned(),
                submission_id: submission_id.as_str().to_owned(),
            },
        })
    }

    pub async fn request(
        &self,
        context: FleetInvocationContext,
        args: AgentRequestArgs,
    ) -> Result<RequestResult, AgentApiError> {
        validate_request_args(&args)?;
        let target_session_id = self
            .resolve_send_target(&context, &args.to)
            .await?
            .ok_or_else(|| AgentApiError::rejected("calling session has no Fleet parent"))?;
        if !self
            .has_session_link_edge(&context.parent_session_id, &target_session_id)
            .await?
        {
            return Err(AgentApiError::rejected(format!(
                "session {target_session_id} is not directly linked to {}",
                context.parent_session_id
            )));
        }
        let submission_id = request_submission_id(&context, &target_session_id);
        let promise_id = request_promise_id(&context, &target_session_id);
        let notify_intents = vec![RunTerminalNotifyIntent {
            holder_workflow_id: self
                .runtime
                .holder_workflow_id(&context.parent_session_id)
                .await?,
            token: promise_id.as_str().to_owned(),
        }];
        let run_id = self
            .runtime
            .enqueue_run(
                &target_session_id,
                request_run_input(&context, &args)?,
                submission_id.clone(),
                notify_intents,
            )
            .await?;
        let target_run_id = parse_run_number(&run_id)?;
        let promise_effect = promise_create_effect(
            &promise_id,
            &PromiseSource::Run {
                target_session_id: target_session_id.as_str().to_owned(),
                target_run_id,
            },
            None,
        );
        Ok(RequestResult {
            output: AgentRequestOutput {
                target_session_id: target_session_id.as_str().to_owned(),
                run_id,
                submission_id: submission_id.as_str().to_owned(),
                promise: promise_id.as_str().to_owned(),
            },
            promise_effect: Some(promise_effect),
        })
    }

    pub async fn await_promises(
        &self,
        context: &FleetInvocationContext,
        _call_id: ToolCallId,
        args: AwaitArgs,
    ) -> Result<engine::AwaitSpec, AgentApiError> {
        await_spec_from_args(args, context.observed_at_ms)
    }

    pub async fn list(
        &self,
        context: FleetInvocationContext,
        args: AgentListArgs,
    ) -> Result<AgentListOutput, AgentApiError> {
        let target_session_id = match args.target_session_id.as_deref() {
            Some(session_id) => parse_session_id(session_id, "target_session_id")?,
            None => context.parent_session_id,
        };
        self.load_session_required(&target_session_id).await?;
        let limit = bounded_list_limit(args.limit)?;
        let link_direction = match args.direction {
            AgentListDirection::Children => SessionLinkDirection::Outgoing,
            AgentListDirection::Parents => SessionLinkDirection::Incoming,
        };
        let links = self
            .sessions
            .list_links(ListSessionLinks {
                session_id: target_session_id.clone(),
                direction: link_direction,
                relationship: Some(FLEET_CHILD_RELATIONSHIP.to_owned()),
                limit,
            })
            .await
            .map_err(map_session_store_error)?;

        let mut agents = Vec::with_capacity(links.len());
        for link in links {
            let session_id = match args.direction {
                AgentListDirection::Children => link.to_session_id.clone(),
                AgentListDirection::Parents => link.from_session_id.clone(),
            };
            let record = self.load_session_required(&session_id).await?;
            let session = self.runtime.read_session(&session_id).await?;
            agents.push(AgentListItem {
                session_id: session_id.as_str().to_owned(),
                relationship: link.relationship,
                created_at_ms: link.created_at_ms,
                status: Some(api_status_name(&session.status)),
                active_run_id: active_run_id(&session),
                updated_at_ms: Some(record.updated_at_ms),
                lineage: lineage_view(&record),
            });
        }

        Ok(AgentListOutput {
            target_session_id: target_session_id.as_str().to_owned(),
            direction: args.direction,
            agents,
        })
    }

    pub async fn read(&self, args: AgentReadArgs) -> Result<AgentReadOutput, AgentApiError> {
        let target_session_id = parse_session_id(&args.target_session_id, "target_session_id")?;
        let record = self.load_session_required(&target_session_id).await?;
        let session = self.runtime.read_session(&target_session_id).await?;
        let environments = serde_json::json!({
            "activeEnvironmentId": session.active_environment_id.clone(),
        });
        let links = self.direct_links(&target_session_id).await?;
        let recent_event_limit = recent_event_limit(args.recent_events.as_ref())?;
        let recent_transcript_limit = recent_transcript_limit(args.recent_transcript.as_ref())?;
        let recent_event_after = recent_after(&record, recent_event_limit);
        let recent_transcript_after = recent_after(&record, recent_transcript_limit);
        let recent_events = self
            .runtime
            .read_session_events(&target_session_id, recent_event_after, recent_event_limit)
            .await?;
        let recent_transcript = self
            .runtime
            .read_session_events(
                &target_session_id,
                recent_transcript_after,
                recent_transcript_limit,
            )
            .await?;

        Ok(AgentReadOutput {
            session_id: target_session_id.as_str().to_owned(),
            session: to_json_value(session)?,
            lineage: lineage_view(&record),
            links,
            environments,
            recent_events: to_json_values(recent_events.events)?,
            recent_transcript: to_json_values(transcript_events(
                recent_transcript.events,
                args.recent_transcript.as_ref(),
            ))?,
        })
    }

    async fn direct_links(
        &self,
        target_session_id: &SessionId,
    ) -> Result<Vec<AgentLinkView>, AgentApiError> {
        let mut links = Vec::new();
        for direction in [
            SessionLinkDirection::Outgoing,
            SessionLinkDirection::Incoming,
        ] {
            let records = self
                .sessions
                .list_links(ListSessionLinks {
                    session_id: target_session_id.clone(),
                    direction,
                    relationship: None,
                    limit: MAX_DIRECT_LINKS,
                })
                .await
                .map_err(map_session_store_error)?;
            links.extend(records.into_iter().map(link_view));
        }
        links.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.from_session_id.cmp(&right.from_session_id))
                .then_with(|| left.to_session_id.cmp(&right.to_session_id))
                .then_with(|| left.relationship.cmp(&right.relationship))
        });
        Ok(links)
    }

    async fn resolve_send_target(
        &self,
        context: &FleetInvocationContext,
        to: &AgentSendTarget,
    ) -> Result<Option<SessionId>, AgentApiError> {
        match to {
            AgentSendTarget::Session { target_session_id } => Ok(Some(parse_session_id(
                target_session_id,
                "target_session_id",
            )?)),
            AgentSendTarget::Parent => self.parent_session_id(&context.parent_session_id).await,
        }
    }

    async fn parent_session_id(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionId>, AgentApiError> {
        let mut links = self
            .sessions
            .list_links(ListSessionLinks {
                session_id: session_id.clone(),
                direction: SessionLinkDirection::Incoming,
                relationship: Some(FLEET_CHILD_RELATIONSHIP.to_owned()),
                limit: MAX_DIRECT_LINKS,
            })
            .await
            .map_err(map_session_store_error)?;
        links.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.from_session_id.cmp(&right.from_session_id))
                .then_with(|| left.to_session_id.cmp(&right.to_session_id))
        });
        Ok(links.into_iter().next().map(|link| link.from_session_id))
    }

    async fn has_session_link_edge(
        &self,
        left: &SessionId,
        right: &SessionId,
    ) -> Result<bool, AgentApiError> {
        if left == right {
            return Ok(false);
        }
        for direction in [
            SessionLinkDirection::Outgoing,
            SessionLinkDirection::Incoming,
        ] {
            let links = self
                .sessions
                .list_links(ListSessionLinks {
                    session_id: left.clone(),
                    direction,
                    relationship: None,
                    limit: MAX_DIRECT_LINKS,
                })
                .await
                .map_err(map_session_store_error)?;
            if links.into_iter().any(|link| {
                (link.from_session_id == *left && link.to_session_id == *right)
                    || (link.from_session_id == *right && link.to_session_id == *left)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn resolve_base_source(
        &self,
        context: &FleetInvocationContext,
        base: &AgentSpawnBase,
    ) -> Result<SessionId, AgentApiError> {
        match base {
            AgentSpawnBase::Self_ { .. } => Ok(context.parent_session_id.clone()),
            AgentSpawnBase::Session { session_id, .. } => {
                parse_session_id(session_id, "base.session_id")
            }
            AgentSpawnBase::Profile { .. } => Err(AgentApiError::invalid_request(
                "profile base does not resolve to a source session",
            )),
        }
    }

    async fn load_session_required(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionRecord, AgentApiError> {
        self.sessions
            .load_session(session_id)
            .await
            .map_err(map_session_store_error)?
            .ok_or_else(|| AgentApiError::not_found(format!("session not found: {session_id}")))
    }

    async fn create_or_reuse_child(
        &self,
        context: &FleetInvocationContext,
        source_record: &SessionRecord,
        child_session_id: &SessionId,
        source_seq: Option<EventSeq>,
        spawn_request_id: &str,
        child_id_was_derived: bool,
    ) -> Result<ChildCreateOutcome, AgentApiError> {
        let result = if let Some(source_seq) = source_seq {
            self.sessions
                .create_forked_session(CreateForkedSession {
                    source_session_id: source_record.session_id.clone(),
                    session_id: child_session_id.clone(),
                    source_seq,
                    created_at_ms: context.observed_at_ms,
                })
                .await
        } else {
            // Cloning is an intentional cold-path replay boundary. The
            // resulting opening events contain only the cloneable seed.
            let entries = api_projection::read_all_session_entries(
                self.sessions.as_ref(),
                &source_record.session_id,
                api_projection::MAX_EVENT_PAGE_LIMIT as usize,
            )
            .await?;
            let state = api_projection::replay_core_agent_state(&entries)?;
            let opening_events = core_agent_clone_opening_events(&state, context.observed_at_ms)
                .map_err(|error| AgentApiError::invalid_request(error.to_string()))?;
            self.sessions
                .create_cloned_session(CreateClonedSession {
                    source_session_id: source_record.session_id.clone(),
                    session_id: child_session_id.clone(),
                    created_at_ms: context.observed_at_ms,
                    opening_events,
                })
                .await
        };

        match result {
            Ok(_) => Ok(ChildCreateOutcome::Created),
            Err(SessionStoreError::SessionAlreadyExists { .. }) => {
                let existing = self
                    .validate_existing_child(
                        child_session_id,
                        &source_record.session_id,
                        source_seq,
                        spawn_request_id,
                        child_id_was_derived,
                    )
                    .await?;
                Ok(ChildCreateOutcome::Reused {
                    matching_spawn_link: existing.matching_spawn_link,
                })
            }
            Err(error) => Err(map_session_store_error(error)),
        }
    }

    async fn validate_existing_child(
        &self,
        child_session_id: &SessionId,
        source_session_id: &SessionId,
        source_seq: Option<EventSeq>,
        spawn_request_id: &str,
        child_id_was_derived: bool,
    ) -> Result<ExistingChildValidation, AgentApiError> {
        let existing = self.load_session_required(child_session_id).await?;
        if existing.source_session_id.as_ref() != Some(source_session_id) {
            return Err(AgentApiError::conflict(format!(
                "child session id {child_session_id} already exists with a different source"
            )));
        }
        if existing.source_seq != source_seq {
            return Err(AgentApiError::conflict(format!(
                "child session id {child_session_id} already exists with a different fork point"
            )));
        }
        let links = self
            .sessions
            .list_links(ListSessionLinks {
                session_id: child_session_id.clone(),
                direction: SessionLinkDirection::Incoming,
                relationship: Some(FLEET_CHILD_RELATIONSHIP.to_owned()),
                limit: 100,
            })
            .await
            .map_err(map_session_store_error)?;
        if links.is_empty() {
            if child_id_was_derived {
                return Ok(ExistingChildValidation {
                    matching_spawn_link: false,
                });
            }
            return Err(AgentApiError::conflict(format!(
                "child session id {child_session_id} already exists without matching fleet spawn metadata"
            )));
        }
        if links.iter().any(|link| {
            link.metadata
                .get("spawn_request_id")
                .and_then(Value::as_str)
                == Some(spawn_request_id)
        }) {
            return Ok(ExistingChildValidation {
                matching_spawn_link: true,
            });
        }
        Err(AgentApiError::conflict(format!(
            "child session id {child_session_id} is already linked to a different spawn request"
        )))
    }

    async fn upsert_spawn_link(
        &self,
        context: &FleetInvocationContext,
        source_session_id: Option<&SessionId>,
        child_session_id: &SessionId,
        source_seq: Option<EventSeq>,
        spawn_request_id: &str,
        args: &AgentSpawnArgs,
    ) -> Result<(), AgentApiError> {
        self.sessions
            .upsert_link(UpsertSessionLink {
                from_session_id: context.parent_session_id.clone(),
                to_session_id: child_session_id.clone(),
                relationship: FLEET_CHILD_RELATIONSHIP.to_owned(),
                created_at_ms: context.observed_at_ms,
                metadata: spawn_link_metadata(
                    context,
                    source_session_id,
                    source_seq,
                    spawn_request_id,
                    args,
                ),
            })
            .await
            .map_err(map_session_store_error)?;
        Ok(())
    }

    async fn apply_resource_policies(
        &self,
        _source_session_id: &SessionId,
        child_session_id: &SessionId,
        observed_at_ms: u64,
        args: &AgentSpawnArgs,
    ) -> Result<(), AgentApiError> {
        if args.environment != EnvironmentPolicy::Share {
            return Err(AgentApiError::invalid_request(
                "agent_spawn environment policy must be share",
            ));
        }
        match args.vfs {
            VfsPolicy::Share => Ok(()),
            VfsPolicy::Isolate => {
                self.isolate_workspace_links(child_session_id, observed_at_ms)
                    .await
            }
        }
    }

    async fn isolate_workspace_links(
        &self,
        child_session_id: &SessionId,
        observed_at_ms: u64,
    ) -> Result<(), AgentApiError> {
        let workspace_store = self.workspace_store.as_ref().ok_or_else(|| {
            AgentApiError::internal("agent_spawn vfs isolate requires a workspace store")
        })?;
        let created_at_ms = nonnegative_i64(observed_at_ms, "observed_at_ms")?;
        let record = self
            .sessions
            .load_session(child_session_id)
            .await
            .map_err(map_session_store_error)?
            .ok_or_else(|| {
                AgentApiError::not_found(format!("session not found: {child_session_id}"))
            })?;
        // VFS isolation is part of child creation and may reconstruct the
        // just-created child. It is not reachable from ordinary tool calls.
        let entries = api_projection::read_all_session_entries(
            self.sessions.as_ref(),
            child_session_id,
            api_projection::MAX_EVENT_PAGE_LIMIT as usize,
        )
        .await?;
        let state = api_projection::replay_core_agent_state(&entries)?;
        let mut config =
            state.lifecycle.config.clone().ok_or_else(|| {
                AgentApiError::internal("isolated child session is missing config")
            })?;
        let Some(vfs) = config.features.vfs.as_mut() else {
            return Ok(());
        };
        let mut changed = false;
        for link in &mut vfs.workspace_links {
            let engine::WorkspaceLinkTarget::Workspace { workspace_id } = &link.target else {
                continue;
            };
            let workspace_id = VfsWorkspaceId::try_new(workspace_id.clone()).map_err(|error| {
                AgentApiError::invalid_request(format!("invalid linked workspace id: {error}"))
            })?;
            let link_path = VfsPath::parse(&link.path).map_err(|error| {
                AgentApiError::invalid_request(format!("invalid workspace link path: {error}"))
            })?;
            let child_workspace_id = isolated_workspace_id(child_session_id, &link_path);
            if workspace_id == child_workspace_id {
                continue;
            }
            let source_workspace = workspace_store
                .read_workspace(&workspace_id)
                .await
                .map_err(map_vfs_catalog_error)?;
            match workspace_store
                .create_workspace(CreateVfsWorkspaceRecord {
                    workspace_id: child_workspace_id.clone(),
                    display_name: None,
                    base_snapshot_ref: Some(source_workspace.head_snapshot_ref.clone()),
                    head_snapshot_ref: source_workspace.head_snapshot_ref,
                    head_totals: source_workspace.head_totals,
                    created_at_ms,
                })
                .await
            {
                Ok(_) | Err(VfsCatalogError::AlreadyExists { .. }) => {}
                Err(error) => return Err(map_vfs_catalog_error(error)),
            }
            link.target = engine::WorkspaceLinkTarget::Workspace {
                workspace_id: child_workspace_id.to_string(),
            };
            changed = true;
        }
        if changed {
            let mut drive = CoreAgentDrive::from_replayed(
                child_session_id.clone(),
                state.clone(),
                record.head.clone(),
            );
            let action = drive
                .admit_command(
                    CoreAgentCommand::ReplaceSessionConfig {
                        expected_revision: Some(state.lifecycle.config_revision),
                        config,
                    },
                    observed_at_ms,
                )
                .map_err(|error| AgentApiError::internal(error.to_string()))?;
            let CoreAgentAction::AppendEvents {
                expected_head,
                events,
            } = action
            else {
                return Err(AgentApiError::internal(
                    "isolating workspace links produced no config event",
                ));
            };
            self.sessions
                .append(AppendSessionEvents {
                    session_id: child_session_id.clone(),
                    expected_head,
                    events,
                })
                .await
                .map_err(map_session_store_error)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildCreateOutcome {
    Created,
    Reused { matching_spawn_link: bool },
}

impl ChildCreateOutcome {
    const fn has_matching_spawn_link(self) -> bool {
        matches!(
            self,
            ChildCreateOutcome::Reused {
                matching_spawn_link: true
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExistingChildValidation {
    matching_spawn_link: bool,
}

/// Result of a spawn: the model-visible output plus, when a run started, the
/// promise-creation effect to attach to the tool call result so the engine
/// records a log-backed promise scoped to the calling run.
#[derive(Clone, Debug)]
pub struct SpawnResult {
    pub output: AgentSpawnOutput,
    pub promise_effect: Option<engine::ToolEffect>,
}

#[derive(Clone, Debug)]
pub struct SendResult {
    pub output: AgentSendOutput,
}

#[derive(Clone, Debug)]
pub struct RequestResult {
    pub output: AgentRequestOutput,
    pub promise_effect: Option<engine::ToolEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FleetInvocationContext {
    pub parent_session_id: SessionId,
    pub parent_run_id: RunId,
    pub turn_id: TurnId,
    pub batch_id: ToolBatchId,
    pub call_id: ToolCallId,
    pub observed_at_ms: u64,
    pub fleet_policy: Option<engine::FleetFeature>,
}

#[async_trait]
pub trait FleetChildRuntime: Send + Sync {
    async fn start_session(
        &self,
        session_id: &SessionId,
        close_on_terminal: bool,
        profile: Option<ProfileSource>,
    ) -> Result<(), AgentApiError>;

    async fn list_profiles(&self) -> Result<Vec<AgentProfileSummary>, AgentApiError>;

    async fn read_profile(&self, profile_id: ProfileId) -> Result<AgentProfile, AgentApiError>;

    async fn start_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError>;

    async fn enqueue_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError>;

    async fn deliver_message(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
    ) -> Result<(), AgentApiError>;

    /// Composed Temporal workflow id of a session in this deployment. Used as
    /// the holder address in a promise notify-intent so the observed child
    /// signals the parent back on terminal.
    async fn holder_workflow_id(&self, session_id: &SessionId) -> Result<String, AgentApiError>;

    async fn read_session(&self, session_id: &SessionId) -> Result<SessionView, AgentApiError>;

    async fn read_session_events(
        &self,
        session_id: &SessionId,
        after: Option<u64>,
        limit: u32,
    ) -> Result<SessionEventsReadResponse, AgentApiError>;
}

#[derive(Clone)]
pub struct AgentApiFleetRuntime {
    api: Arc<GatewayAgentApi>,
}

impl AgentApiFleetRuntime {
    pub fn new(api: Arc<GatewayAgentApi>) -> Self {
        Self { api }
    }
}

#[async_trait]
impl FleetChildRuntime for AgentApiFleetRuntime {
    async fn start_session(
        &self,
        session_id: &SessionId,
        close_on_terminal: bool,
        profile: Option<ProfileSource>,
    ) -> Result<(), AgentApiError> {
        self.api
            .start_session_for_fleet_with_profile(session_id, close_on_terminal, profile)
            .await?;
        Ok(())
    }

    async fn list_profiles(&self) -> Result<Vec<AgentProfileSummary>, AgentApiError> {
        let response = self.api.list_profiles(ProfileListParams {}).await?;
        Ok(response.result.profiles)
    }

    async fn read_profile(&self, profile_id: ProfileId) -> Result<AgentProfile, AgentApiError> {
        let response = self
            .api
            .read_profile(ProfileReadParams { profile_id })
            .await?;
        Ok(response.result.profile)
    }

    async fn start_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError> {
        self.api
            .start_run_for_fleet(session_id, input, submission_id, notify_on_terminal)
            .await
    }

    async fn enqueue_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError> {
        self.api
            .enqueue_run_for_fleet(session_id, input, submission_id, notify_on_terminal)
            .await
    }

    async fn deliver_message(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
    ) -> Result<(), AgentApiError> {
        self.api
            .deliver_message_for_fleet(session_id, input, submission_id)
            .await
    }

    async fn holder_workflow_id(&self, session_id: &SessionId) -> Result<String, AgentApiError> {
        Ok(self.api.workflow_id_for(session_id))
    }

    async fn read_session(&self, session_id: &SessionId) -> Result<SessionView, AgentApiError> {
        let response = self
            .api
            .read_session(SessionReadParams {
                session_id: session_id.as_str().to_owned(),
            })
            .await?;
        Ok(response.result.session)
    }

    async fn read_session_events(
        &self,
        session_id: &SessionId,
        after: Option<u64>,
        limit: u32,
    ) -> Result<SessionEventsReadResponse, AgentApiError> {
        let response = self
            .api
            .read_session_events(SessionEventsReadParams {
                session_id: session_id.as_str().to_owned(),
                after: after.map(|seq| EventCursor { seq }),
                limit: Some(limit),
                wait_ms: Some(0),
            })
            .await?;
        Ok(response.result)
    }
}

#[derive(Clone)]
pub struct FleetToolExecutor {
    blobs: Arc<dyn BlobStore>,
    service: FleetService,
}

impl FleetToolExecutor {
    pub fn new(blobs: Arc<dyn BlobStore>, service: FleetService) -> Self {
        Self { blobs, service }
    }

    pub async fn invoke(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        match call.tool_name.as_str() {
            AGENT_SPAWN_TOOL_NAME => self.invoke_spawn(context, call).await,
            AGENT_REQUEST_TOOL_NAME => self.invoke_request(context, call).await,
            AGENT_SEND_TOOL_NAME => self.invoke_send(context, call).await,
            CANCEL_TOOL_NAME => self.invoke_cancel_promises(context, call).await,
            DETACH_TOOL_NAME => self.invoke_detach_promises(context, call).await,
            AWAIT_TOOL_NAME => {
                fleet_failed_result(
                    self.blobs.as_ref(),
                    call.call_id.clone(),
                    "await must be the only call in its tool batch",
                )
                .await
            }
            AGENT_LIST_TOOL_NAME => self.invoke_list(context, call).await,
            AGENT_READ_TOOL_NAME => self.invoke_read(call).await,
            PROFILE_LIST_TOOL_NAME => self.invoke_profile_list(context, call).await,
            PROFILE_READ_TOOL_NAME => self.invoke_profile_read(context, call).await,
            other => {
                fleet_failed_result(
                    self.blobs.as_ref(),
                    call.call_id.clone(),
                    format!("unknown Fleet tool: {other}"),
                )
                .await
            }
        }
    }

    pub async fn invoke_await_batch(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolBatchOutcome, CoreAgentIoError> {
        let args: AwaitArgs = self.decode_args(call).await?;
        match self
            .service
            .await_promises(&context, call.call_id.clone(), args)
            .await
        {
            Ok(spec) => Ok(ToolBatchOutcome::Deferred {
                batch_id: context.batch_id,
                call_id: call.call_id.clone(),
                completed_results: Vec::new(),
                spec,
            }),
            Err(error) => {
                let result = fleet_failed_result(
                    self.blobs.as_ref(),
                    call.call_id.clone(),
                    error.to_string(),
                )
                .await?;
                Ok(ToolBatchOutcome::completed(ToolInvocationBatchResult {
                    run_id: context.parent_run_id,
                    turn_id: context.turn_id,
                    batch_id: context.batch_id,
                    results: vec![result],
                }))
            }
        }
    }

    async fn invoke_spawn(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: AgentSpawnArgs = self.decode_args(call).await?;
        match self.service.spawn(context, args).await {
            Ok(SpawnResult {
                output,
                promise_effect,
            }) => {
                let visible = spawn_model_visible_text(&output);
                let mut result = self
                    .succeeded(call.call_id.clone(), &output, visible)
                    .await?;
                // The promise-creation effect becomes a `Promise(Created)`
                // log event in the same append as this call's completion.
                result.effects.extend(promise_effect);
                Ok(result)
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_send(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: AgentSendArgs = self.decode_args(call).await?;
        match self.service.send(context, args).await {
            Ok(SendResult { output }) => {
                let visible = send_model_visible_text(&output);
                self.succeeded(call.call_id.clone(), &output, visible).await
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_request(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: AgentRequestArgs = self.decode_args(call).await?;
        match self.service.request(context, args).await {
            Ok(RequestResult {
                output,
                promise_effect,
            }) => {
                let visible = request_model_visible_text(&output);
                let mut result = self
                    .succeeded(call.call_id.clone(), &output, visible)
                    .await?;
                result.effects.extend(promise_effect);
                Ok(result)
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_cancel_promises(
        &self,
        _context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: CancelArgs = self.decode_args(call).await?;
        match cancel_promises_from_runtime(&args, call.promise_control.as_ref()) {
            Ok((output, effects)) => {
                let visible = cancel_promises_model_visible_text(&output);
                let mut tool_result = self
                    .succeeded(call.call_id.clone(), &output, visible)
                    .await?;
                tool_result.effects.extend(effects);
                Ok(tool_result)
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_detach_promises(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: DetachArgs = self.decode_args(call).await?;
        match detach_promises_from_runtime(
            &args,
            context.parent_run_id,
            call.promise_control.as_ref(),
        ) {
            Ok((output, effects)) => {
                let visible = detach_promises_model_visible_text(&output);
                let mut tool_result = self
                    .succeeded(call.call_id.clone(), &output, visible)
                    .await?;
                tool_result.effects.extend(effects);
                Ok(tool_result)
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_list(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: AgentListArgs = self.decode_args(call).await?;
        match self.service.list(context, args).await {
            Ok(output) => {
                let visible = list_model_visible_text(&output);
                self.succeeded(call.call_id.clone(), &output, visible).await
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_read(
        &self,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: AgentReadArgs = self.decode_args(call).await?;
        match self.service.read(args).await {
            Ok(output) => {
                self.succeeded_structured(call.call_id.clone(), &output)
                    .await
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_profile_list(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: ProfileListArgs = self.decode_args(call).await?;
        match self.service.list_profiles(&context, args).await {
            Ok(output) => {
                let visible = profile_list_model_visible_text(&output);
                self.succeeded(call.call_id.clone(), &output, visible).await
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn invoke_profile_read(
        &self,
        context: FleetInvocationContext,
        call: &ToolInvocationRequest,
    ) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let args: ProfileReadArgs = self.decode_args(call).await?;
        match self.service.read_profile(&context, args).await {
            Ok(output) => {
                let visible = profile_read_model_visible_text(&output);
                self.succeeded(call.call_id.clone(), &output, visible).await
            }
            Err(error) => {
                fleet_failed_result(self.blobs.as_ref(), call.call_id.clone(), error.to_string())
                    .await
            }
        }
    }

    async fn succeeded<T>(
        &self,
        call_id: ToolCallId,
        output: &T,
        visible: String,
    ) -> Result<ToolInvocationResult, CoreAgentIoError>
    where
        T: Serialize,
    {
        let output_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(output).map_err(io_error)?)
            .await
            .map_err(map_blob_error)?;
        let visible_ref = self
            .blobs
            .put_bytes(visible.into_bytes())
            .await
            .map_err(map_blob_error)?;
        let model_visible_context_entries = vec![ToolInvocationResult::tool_result_context_entry(
            &call_id,
            ToolCallStatus::Succeeded,
            visible_ref,
        )];
        Ok(ToolInvocationResult {
            call_id,
            status: ToolCallStatus::Succeeded,
            output_ref: Some(output_ref),
            model_visible_context_entries,
            error_ref: None,
            effects: Vec::new(),
        })
    }

    async fn succeeded_structured<T>(
        &self,
        call_id: ToolCallId,
        output: &T,
    ) -> Result<ToolInvocationResult, CoreAgentIoError>
    where
        T: Serialize,
    {
        let output_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(output).map_err(io_error)?)
            .await
            .map_err(map_blob_error)?;
        let model_visible_context_entries = vec![ToolInvocationResult::tool_result_context_entry(
            &call_id,
            ToolCallStatus::Succeeded,
            output_ref.clone(),
        )];
        Ok(ToolInvocationResult {
            call_id,
            status: ToolCallStatus::Succeeded,
            output_ref: Some(output_ref),
            model_visible_context_entries,
            error_ref: None,
            effects: Vec::new(),
        })
    }

    async fn decode_args<T>(&self, call: &ToolInvocationRequest) -> Result<T, CoreAgentIoError>
    where
        T: serde::de::DeserializeOwned,
    {
        let bytes = self
            .blobs
            .read_bytes(&call.arguments_ref)
            .await
            .map_err(map_blob_error)?;
        serde_json::from_slice(&bytes)
            .map_err(|error| io_error(format!("invalid JSON tool arguments: {error}")))
    }
}

fn validate_spawn_args(args: &AgentSpawnArgs) -> Result<(), AgentApiError> {
    if args.input.trim().is_empty() {
        return Err(AgentApiError::invalid_request(
            "agent_spawn input must not be empty",
        ));
    }
    if args.base.profile().is_some() && args.vfs != VfsPolicy::Share {
        return Err(AgentApiError::invalid_request(
            "agent_spawn profile requires vfs=share",
        ));
    }
    if args.environment != EnvironmentPolicy::Share {
        return Err(AgentApiError::invalid_request(
            "agent_spawn environment policy must be share",
        ));
    }
    if args.lifecycle.close_on_terminal && !args.lifecycle.run_immediately {
        return Err(AgentApiError::invalid_request(
            "agent_spawn lifecycle.close_on_terminal requires lifecycle.run_immediately",
        ));
    }
    Ok(())
}

fn fleet_policy(context: &FleetInvocationContext) -> Result<&engine::FleetFeature, AgentApiError> {
    context.fleet_policy.as_ref().ok_or_else(|| {
        AgentApiError::invalid_request(
            "Fleet tool invocation is missing its admitted parent policy",
        )
    })
}

fn validate_spawn_policy(
    fleet_config: &engine::FleetFeature,
    args: &AgentSpawnArgs,
) -> Result<(), AgentApiError> {
    let base = match &args.base {
        AgentSpawnBase::Self_ { .. } => engine::FleetSpawnBase::Self_,
        AgentSpawnBase::Session { .. } => engine::FleetSpawnBase::Session,
        AgentSpawnBase::Profile { .. } => engine::FleetSpawnBase::Profile,
    };
    if !fleet_config.spawn.base_allowed(base) {
        return Err(AgentApiError::invalid_request(format!(
            "agent_spawn base {} is not allowed by this profile",
            fleet_spawn_base_name(base)
        )));
    }
    if let AgentSpawnBase::Profile { profile } = &args.base {
        validate_profile_source_allowed(fleet_config, profile)?;
    }
    Ok(())
}

fn validate_profile_source_allowed(
    fleet_config: &engine::FleetFeature,
    profile: &ProfileSource,
) -> Result<(), AgentApiError> {
    match profile {
        ProfileSource::Named { profile_id } => {
            validate_named_profile_allowed(fleet_config, profile_id)
        }
        ProfileSource::Inline { .. } if fleet_config.profiles.inline => Ok(()),
        ProfileSource::Inline { .. } => Err(AgentApiError::invalid_request(
            "inline profiles are not allowed by this profile",
        )),
    }
}

fn validate_named_profile_allowed(
    fleet_config: &engine::FleetFeature,
    profile_id: &ProfileId,
) -> Result<(), AgentApiError> {
    if fleet_config
        .profiles
        .named_profile_allowed(profile_id.as_str())
    {
        Ok(())
    } else {
        Err(AgentApiError::invalid_request(format!(
            "profile {profile_id} is not allowed by this profile"
        )))
    }
}

fn fleet_spawn_base_name(base: engine::FleetSpawnBase) -> &'static str {
    match base {
        engine::FleetSpawnBase::Self_ => "self",
        engine::FleetSpawnBase::Session => "session",
        engine::FleetSpawnBase::Profile => "profile",
    }
}

fn validate_send_args(args: &AgentSendArgs) -> Result<(), AgentApiError> {
    if args.text.trim().is_empty() {
        return Err(AgentApiError::invalid_request(
            "agent_send text must not be empty",
        ));
    }
    Ok(())
}

fn validate_request_args(args: &AgentRequestArgs) -> Result<(), AgentApiError> {
    if args.text.trim().is_empty() {
        return Err(AgentApiError::invalid_request(
            "agent_request text must not be empty",
        ));
    }
    Ok(())
}

pub(crate) fn await_spec_from_args(
    args: AwaitArgs,
    observed_at_ms: u64,
) -> Result<engine::AwaitSpec, AgentApiError> {
    let promise_ids = validate_await_args(&args)?;
    Ok(engine::AwaitSpec {
        promise_ids,
        mode: match args.mode {
            AwaitModeArg::All => engine::AwaitMode::All,
            AwaitModeArg::Any => engine::AwaitMode::Any,
        },
        mailbox: args.mailbox,
        deadline_at_ms: args
            .timeout_ms
            .map(|timeout| observed_at_ms.saturating_add(timeout)),
    })
}

fn validate_await_args(args: &AwaitArgs) -> Result<Vec<PromiseId>, AgentApiError> {
    Ok(args
        .validated_promise_ids()
        .map_err(|error| AgentApiError::invalid_request(error.to_string()))?
        .iter()
        .map(|promise_id| PromiseId::new(promise_id.clone()))
        .collect())
}

fn bounded_list_limit(limit: Option<u32>) -> Result<usize, AgentApiError> {
    let limit = limit.unwrap_or(DEFAULT_AGENT_LIST_LIMIT as u32);
    if limit == 0 {
        return Err(AgentApiError::invalid_request(
            "agent_list limit must be at least 1",
        ));
    }
    Ok((limit as usize).min(MAX_AGENT_LIST_LIMIT))
}

fn recent_event_limit(
    selector: Option<&tools::fleet::RecentEventsSelector>,
) -> Result<u32, AgentApiError> {
    let limit = selector.map_or(DEFAULT_RECENT_EVENT_LIMIT, |selector| selector.limit);
    bounded_recent_limit(limit, "recent_events.limit")
}

fn recent_transcript_limit(
    selector: Option<&tools::fleet::RecentTranscriptSelector>,
) -> Result<u32, AgentApiError> {
    let Some(selector) = selector else {
        return Ok(DEFAULT_RECENT_TRANSCRIPT_EVENT_LIMIT);
    };
    let limit = match (selector.events, selector.turns) {
        (Some(events), _) => events,
        (None, Some(_)) => MAX_RECENT_EVENT_LIMIT,
        (None, None) => DEFAULT_RECENT_TRANSCRIPT_EVENT_LIMIT,
    };
    bounded_recent_limit(limit, "recent_transcript")
}

fn bounded_recent_limit(limit: u32, field: &str) -> Result<u32, AgentApiError> {
    if limit == 0 {
        return Err(AgentApiError::invalid_request(format!(
            "{field} must be at least 1"
        )));
    }
    Ok(limit.min(MAX_RECENT_EVENT_LIMIT))
}

fn recent_after(record: &SessionRecord, limit: u32) -> Option<u64> {
    let head_seq = record.head.as_ref()?.seq.as_u64();
    let after = head_seq.saturating_sub(limit as u64);
    (after > 0).then_some(after)
}

fn active_run_id(session: &SessionView) -> Option<String> {
    session
        .runs
        .iter()
        .find(|run| matches!(run.status, ApiRunStatus::Running))
        .map(|run| run.id.clone())
}

fn lineage_view(record: &SessionRecord) -> AgentLineageView {
    AgentLineageView {
        source_session_id: record
            .source_session_id
            .as_ref()
            .map(|session_id| session_id.as_str().to_owned()),
        source_seq: record.source_seq.map(EventSeq::as_u64),
    }
}

fn link_view(record: engine::storage::SessionLinkRecord) -> AgentLinkView {
    AgentLinkView {
        from_session_id: record.from_session_id.as_str().to_owned(),
        to_session_id: record.to_session_id.as_str().to_owned(),
        relationship: record.relationship,
        created_at_ms: record.created_at_ms,
        metadata: record.metadata,
    }
}

fn api_status_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn to_json_value<T: Serialize>(value: T) -> Result<Value, AgentApiError> {
    serde_json::to_value(value)
        .map_err(|error| AgentApiError::internal(format!("failed to encode Fleet output: {error}")))
}

fn to_json_values<T: Serialize>(values: Vec<T>) -> Result<Vec<Value>, AgentApiError> {
    values.into_iter().map(to_json_value).collect()
}

fn transcript_events(
    events: Vec<api::SessionEventView>,
    selector: Option<&tools::fleet::RecentTranscriptSelector>,
) -> Vec<api::SessionEventView> {
    let Some(turns) = selector.and_then(|selector| selector.turns) else {
        return events;
    };
    if selector.and_then(|selector| selector.events).is_some() {
        return events;
    }
    let mut selected_turn_ids = Vec::new();
    for event in events.iter().rev() {
        let Some(turn_id) = event.joins.turn_id.as_deref() else {
            continue;
        };
        if selected_turn_ids.iter().any(|selected| selected == turn_id) {
            continue;
        }
        selected_turn_ids.push(turn_id.to_owned());
        if selected_turn_ids.len() >= turns as usize {
            break;
        }
    }
    if selected_turn_ids.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|event| {
            event
                .joins
                .turn_id
                .as_ref()
                .is_some_and(|turn_id| selected_turn_ids.contains(turn_id))
        })
        .collect()
}

fn parse_session_id(value: &str, field: &str) -> Result<SessionId, AgentApiError> {
    SessionId::try_new(value.to_owned())
        .map_err(|error| AgentApiError::invalid_request(format!("invalid {field}: {error}")))
}

fn derived_child_session_id(context: &FleetInvocationContext) -> SessionId {
    let digest = digest_suffix(&spawn_request_material(context));
    SessionId::new(format!("agent_{digest}"))
}

fn spawn_request_id(context: &FleetInvocationContext) -> String {
    format!(
        "fleet_spawn_{}",
        digest_suffix(&spawn_request_material(context))
    )
}

fn spawn_promise_id(context: &FleetInvocationContext, child_session_id: &SessionId) -> String {
    format!(
        "promise_{}",
        digest_suffix(&format!(
            "{}:{}",
            spawn_request_material(context),
            child_session_id
        ))
    )
}

fn request_promise_id(
    context: &FleetInvocationContext,
    target_session_id: &SessionId,
) -> PromiseId {
    PromiseId::new(format!(
        "promise_request_{}",
        digest_suffix(&format!(
            "{}:{}",
            spawn_request_material(context),
            target_session_id
        ))
    ))
}

fn parse_run_number(run_id: &str) -> Result<u64, AgentApiError> {
    let Some(number) = run_id.strip_prefix("run_") else {
        return Err(AgentApiError::internal(format!(
            "fleet runtime returned malformed run id: {run_id}"
        )));
    };
    number.parse::<u64>().map_err(|_| {
        AgentApiError::internal(format!("fleet runtime returned malformed run id: {run_id}"))
    })
}

/// Parse an api run id (`run_<n>`) into its numeric id. A malformed id would
/// only arise from an internal invariant break; fall back to 0 rather than
/// panic, since the promise is still correlated by its stable id.
fn parse_api_run_id_u64(run_id: &str) -> u64 {
    run_id
        .strip_prefix("run_")
        .and_then(|rest| rest.parse::<u64>().ok())
        .unwrap_or(0)
}

fn child_run_submission_id(context: &FleetInvocationContext) -> SubmissionId {
    SubmissionId::new(format!(
        "fleet_run_{}",
        digest_suffix(&spawn_request_material(context))
    ))
}

fn send_submission_id(
    context: &FleetInvocationContext,
    target_session_id: &SessionId,
) -> SubmissionId {
    SubmissionId::new(format!(
        "fleet_send_{}",
        digest_suffix(&format!(
            "{}:{}",
            spawn_request_material(context),
            target_session_id
        ))
    ))
}

fn request_submission_id(
    context: &FleetInvocationContext,
    target_session_id: &SessionId,
) -> SubmissionId {
    SubmissionId::new(format!(
        "fleet_request_{}",
        digest_suffix(&format!(
            "{}:{}",
            spawn_request_material(context),
            target_session_id
        ))
    ))
}

fn spawn_request_material(context: &FleetInvocationContext) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        context.parent_session_id,
        context.parent_run_id,
        context.turn_id,
        context.batch_id,
        context.call_id
    )
}

fn digest_suffix(value: &str) -> String {
    let digest = BlobRef::from_bytes(value.as_bytes());
    digest
        .as_str()
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .chars()
        .take(32)
        .collect()
}

fn isolated_workspace_id(child_session_id: &SessionId, mount_path: &VfsPath) -> VfsWorkspaceId {
    let digest = digest_suffix(&format!("{child_session_id}:{}", mount_path.as_str()));
    VfsWorkspaceId::new(format!("workspace_{digest}"))
}

fn spawn_run_text(_context: &FleetInvocationContext, args: &AgentSpawnArgs) -> String {
    args.input.clone()
}

fn send_run_input(
    context: &FleetInvocationContext,
    args: &AgentSendArgs,
) -> Result<Vec<InputItem>, AgentApiError> {
    let envelope = fleet_send_envelope_text(context, args.text.trim(), args.payload.clone())?;
    let mut input = Vec::with_capacity(args.input.len() + 1);
    input.push(InputItem::Text { text: envelope });
    input.extend(
        args.input
            .iter()
            .map(send_input_item_to_api)
            .collect::<Vec<_>>(),
    );
    Ok(input)
}

fn request_run_input(
    context: &FleetInvocationContext,
    args: &AgentRequestArgs,
) -> Result<Vec<InputItem>, AgentApiError> {
    let envelope = fleet_request_envelope_text(context, args.text.trim(), args.payload.clone())?;
    let mut input = Vec::with_capacity(args.input.len() + 1);
    input.push(InputItem::Text { text: envelope });
    input.extend(
        args.input
            .iter()
            .map(send_input_item_to_api)
            .collect::<Vec<_>>(),
    );
    Ok(input)
}

fn send_input_item_to_api(item: &AgentSendInputItem) -> InputItem {
    match item {
        AgentSendInputItem::Text { text } => InputItem::Text { text: text.clone() },
        AgentSendInputItem::TextRef { blob_ref } => InputItem::TextRef {
            blob_ref: blob_ref.clone(),
        },
        AgentSendInputItem::Media {
            blob_ref,
            mime,
            kind,
            name,
        } => InputItem::Media {
            blob_ref: blob_ref.clone(),
            mime: mime.clone(),
            kind: send_media_kind_to_api(*kind),
            name: name.clone(),
        },
    }
}

fn send_media_kind_to_api(kind: AgentSendMediaKind) -> MediaKind {
    match kind {
        AgentSendMediaKind::Image => MediaKind::Image,
        AgentSendMediaKind::Audio => MediaKind::Audio,
        AgentSendMediaKind::Document => MediaKind::Document,
    }
}

fn fleet_send_envelope_text(
    context: &FleetInvocationContext,
    raw_text: &str,
    payload: Option<Value>,
) -> Result<String, AgentApiError> {
    let mut fleet_send = serde_json::Map::new();
    fleet_send.insert(
        "from_session_id".to_owned(),
        Value::String(context.parent_session_id.as_str().to_owned()),
    );
    if let Some(payload) = payload {
        fleet_send.insert("payload".to_owned(), payload);
    }
    serde_json::to_string(&json!({
        "fleet_send": Value::Object(fleet_send),
        "text": raw_text,
    }))
    .map_err(|error| {
        AgentApiError::internal(format!("failed to encode Fleet send envelope: {error}"))
    })
}

fn fleet_request_envelope_text(
    context: &FleetInvocationContext,
    raw_text: &str,
    payload: Option<Value>,
) -> Result<String, AgentApiError> {
    let mut fleet_request = serde_json::Map::new();
    fleet_request.insert(
        "from_session_id".to_owned(),
        Value::String(context.parent_session_id.as_str().to_owned()),
    );
    if let Some(payload) = payload {
        fleet_request.insert("payload".to_owned(), payload);
    }
    serde_json::to_string(&json!({
        "fleet_request": Value::Object(fleet_request),
        "text": raw_text,
    }))
    .map_err(|error| {
        AgentApiError::internal(format!("failed to encode Fleet request envelope: {error}"))
    })
}

fn spawn_link_metadata(
    context: &FleetInvocationContext,
    source_session_id: Option<&SessionId>,
    source_seq: Option<EventSeq>,
    spawn_request_id: &str,
    args: &AgentSpawnArgs,
) -> Value {
    json!({
        "kind": "fleet_spawn",
        "spawn_request_id": spawn_request_id,
        "parent_run_id": context.parent_run_id.as_u64(),
        "turn_id": context.turn_id.as_u64(),
        "tool_batch_id": context.batch_id.as_u64(),
        "tool_call_id": context.call_id.as_str(),
        "source_session_id": source_session_id.map(SessionId::as_str),
        "source_seq": source_seq.map(EventSeq::as_u64),
        "base": &args.base,
        "profile": args.base.profile(),
        "fork": args.base.fork().is_some(),
        "fork_at_seq": args.base.fork().and_then(|fork| match fork {
            AgentSpawnFork::Safe => None,
            AgentSpawnFork::AtSeq { seq } => Some(*seq),
        }),
        "vfs": match args.vfs {
            VfsPolicy::Share => "share",
            VfsPolicy::Isolate => "isolate",
        },
        "environment": "share",
    })
}

fn map_session_store_error(error: SessionStoreError) -> AgentApiError {
    match error {
        SessionStoreError::SessionAlreadyExists { session_id } => {
            AgentApiError::conflict(format!("session already exists: {session_id}"))
        }
        SessionStoreError::SessionNotFound { session_id } => {
            AgentApiError::not_found(format!("session not found: {session_id}"))
        }
        SessionStoreError::ExpectedHeadMismatch { .. } => {
            AgentApiError::conflict(error.to_string())
        }
        SessionStoreError::InvalidForkPoint { .. }
        | SessionStoreError::InvalidRelationship { .. }
        | SessionStoreError::InvalidLimit { .. } => {
            AgentApiError::invalid_request(error.to_string())
        }
        SessionStoreError::SessionNotClosed { .. } => AgentApiError::rejected(error.to_string()),
        SessionStoreError::ManagedSessionCannotBranch { .. } => {
            AgentApiError::rejected(error.to_string())
        }
        SessionStoreError::SessionHasForkChildren { .. } => {
            AgentApiError::conflict(error.to_string())
        }
        SessionStoreError::Store { .. } => AgentApiError::internal(error.to_string()),
    }
}

fn map_vfs_catalog_error(error: VfsCatalogError) -> AgentApiError {
    match error {
        VfsCatalogError::AlreadyExists { .. } | VfsCatalogError::RevisionConflict { .. } => {
            AgentApiError::conflict(error.to_string())
        }
        VfsCatalogError::NotFound { .. } => AgentApiError::not_found(error.to_string()),
        VfsCatalogError::InvalidInput { .. } => AgentApiError::invalid_request(error.to_string()),
        VfsCatalogError::Store { .. } => AgentApiError::internal(error.to_string()),
    }
}

fn nonnegative_i64(value: u64, name: &str) -> Result<i64, AgentApiError> {
    i64::try_from(value)
        .map_err(|_| AgentApiError::invalid_request(format!("{name} is too large: {value}")))
}

fn spawn_model_visible_text(output: &AgentSpawnOutput) -> String {
    match (output.child_run_id.as_deref(), output.promise.as_deref()) {
        (Some(run_id), Some(promise)) => format!(
            "Agent {} {} and started run {} (promise {}). Await it with the await tool.",
            output.child_session_id, output.status, run_id, promise
        ),
        (Some(run_id), None) => format!(
            "Agent {} {} and started run {}.",
            output.child_session_id, output.status, run_id
        ),
        (None, _) => format!(
            "Agent {} {} without starting a run.",
            output.child_session_id, output.status
        ),
    }
}

fn send_model_visible_text(output: &AgentSendOutput) -> String {
    format!(
        "Delivered message to session {} (submission {}).",
        output.target_session_id, output.submission_id
    )
}

fn request_model_visible_text(output: &AgentRequestOutput) -> String {
    format!(
        "Requested work from session {} as run {} (submission {}, promise {}). Await it with the await tool.",
        output.target_session_id, output.run_id, output.submission_id, output.promise
    )
}

fn list_model_visible_text(output: &AgentListOutput) -> String {
    format!(
        "Found {} {} agent(s) for {}.",
        output.agents.len(),
        match output.direction {
            AgentListDirection::Children => "child",
            AgentListDirection::Parents => "parent",
        },
        output.target_session_id
    )
}

fn profile_list_model_visible_text(output: &ProfileListOutput) -> String {
    if output.profiles.is_empty() {
        return "No agent profiles are available.".to_owned();
    }
    let mut lines = Vec::with_capacity(output.profiles.len() + 1);
    lines.push(format!("Found {} agent profile(s).", output.profiles.len()));
    for profile in &output.profiles {
        let mut line = format!(
            "- {} (revision {}, updated_at_ms {})",
            profile.profile_id, profile.revision, profile.updated_at_ms
        );
        if let Some(display_name) = profile
            .display_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            line.push_str(": ");
            line.push_str(display_name);
        }
        if let Some(description) = profile
            .description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            line.push_str(" - ");
            line.push_str(description);
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn profile_read_model_visible_text(output: &ProfileReadOutput) -> String {
    let profile = &output.profile;
    format!(
        "Read profile {} revision {}: config {}, instructions {}, active environment {}.",
        profile.profile_id,
        profile.revision,
        yes_no(profile.document.config.is_some()),
        yes_no(profile.document.instructions.is_some()),
        profile
            .document
            .active_environment_id
            .as_deref()
            .unwrap_or("none")
    )
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

async fn fleet_failed_result(
    blobs: &dyn BlobStore,
    call_id: ToolCallId,
    message: impl Into<String>,
) -> Result<ToolInvocationResult, CoreAgentIoError> {
    let error_ref = blobs
        .put_bytes(message.into().into_bytes())
        .await
        .map_err(map_blob_error)?;
    let model_visible_context_entries = vec![ToolInvocationResult::tool_result_context_entry(
        &call_id,
        ToolCallStatus::Failed,
        error_ref.clone(),
    )];
    Ok(ToolInvocationResult {
        call_id,
        status: ToolCallStatus::Failed,
        output_ref: None,
        model_visible_context_entries,
        error_ref: Some(error_ref),
        effects: Vec::new(),
    })
}

fn map_blob_error(error: BlobStoreError) -> CoreAgentIoError {
    io_error(format!("Fleet tool blob operation failed: {error}"))
}

fn io_error(error: impl std::fmt::Display) -> CoreAgentIoError {
    CoreAgentIoError::Failed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use engine::{
        ContextConfig, ContextEntryKind, FeaturesConfig, FleetFeature, FleetProfilesConfig,
        FleetSpawnBase, FleetSpawnConfig, GenerationConfig, LimitsConfig, ModelSelection,
        ProviderApiKind, RunConfig, SessionConfig, ToolBatchOutcome, ToolCallId,
        ToolInvocationRequest, ToolName, WorkspaceLink, WorkspaceLinkAccess, WorkspaceLinkTarget,
        storage::{CreateSession, InMemoryBlobStore, InMemorySessionStore, SessionStore},
    };
    use vfs::{CompareAndSetVfsWorkspaceHead, VfsWorkspaceRecord};

    use super::*;

    fn visible_tool_result_ref(result: &ToolInvocationResult) -> BlobRef {
        result
            .model_visible_context_entries
            .iter()
            .find_map(|entry| {
                matches!(entry.kind, ContextEntryKind::ToolResult { .. })
                    .then(|| entry.content_ref.clone())
            })
            .expect("visible tool result")
    }

    #[derive(Clone)]
    struct StartedRun {
        session_id: SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    }

    #[derive(Default)]
    struct FakeRuntime {
        session_store: Option<Arc<InMemorySessionStore>>,
        started_sessions: Mutex<Vec<(SessionId, bool, Option<ProfileSource>)>>,
        started_runs: Mutex<Vec<StartedRun>>,
        sessions: Mutex<BTreeMap<SessionId, SessionView>>,
        events: Mutex<BTreeMap<SessionId, Vec<api::SessionEventView>>>,
        profiles: Mutex<BTreeMap<ProfileId, AgentProfile>>,
    }

    struct CountingSessionStore {
        inner: Arc<InMemorySessionStore>,
        event_reads: AtomicUsize,
    }

    impl CountingSessionStore {
        fn new(inner: Arc<InMemorySessionStore>) -> Self {
            Self {
                inner,
                event_reads: AtomicUsize::new(0),
            }
        }

        fn event_reads(&self) -> usize {
            self.event_reads.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl SessionStore for CountingSessionStore {
        async fn create_session(
            &self,
            request: engine::storage::CreateSession,
        ) -> Result<engine::storage::SessionRecord, engine::storage::SessionStoreError> {
            self.inner.create_session(request).await
        }

        async fn load_session(
            &self,
            session_id: &SessionId,
        ) -> Result<Option<engine::storage::SessionRecord>, engine::storage::SessionStoreError>
        {
            self.inner.load_session(session_id).await
        }

        async fn list_links(
            &self,
            request: engine::storage::ListSessionLinks,
        ) -> Result<Vec<engine::storage::SessionLinkRecord>, engine::storage::SessionStoreError>
        {
            self.inner.list_links(request).await
        }

        async fn append(
            &self,
            request: engine::storage::AppendSessionEvents,
        ) -> Result<engine::storage::AppendSessionEventsResult, engine::storage::SessionStoreError>
        {
            self.inner.append(request).await
        }

        async fn read_after(
            &self,
            request: engine::storage::ReadSessionEvents,
        ) -> Result<engine::storage::SessionPage, engine::storage::SessionStoreError> {
            self.event_reads.fetch_add(1, Ordering::Relaxed);
            self.inner.read_after(request).await
        }

        async fn head(
            &self,
            session_id: &SessionId,
        ) -> Result<Option<engine::SessionPosition>, engine::storage::SessionStoreError> {
            self.inner.head(session_id).await
        }
    }

    #[async_trait]
    impl FleetChildRuntime for FakeRuntime {
        async fn start_session(
            &self,
            session_id: &SessionId,
            close_on_terminal: bool,
            profile: Option<ProfileSource>,
        ) -> Result<(), AgentApiError> {
            if let Some(store) = &self.session_store
                && store
                    .load_session(session_id)
                    .await
                    .map_err(map_session_store_error)?
                    .is_none()
            {
                store
                    .create_session(CreateSession {
                        session_id: session_id.clone(),
                        display_name: None,
                        created_at_ms: 1,
                    })
                    .await
                    .map_err(map_session_store_error)?;
            }
            self.started_sessions.lock().expect("lock").push((
                session_id.clone(),
                close_on_terminal,
                profile,
            ));
            Ok(())
        }

        async fn list_profiles(&self) -> Result<Vec<AgentProfileSummary>, AgentApiError> {
            Ok(self
                .profiles
                .lock()
                .expect("lock")
                .values()
                .map(AgentProfile::summary)
                .collect())
        }

        async fn read_profile(&self, profile_id: ProfileId) -> Result<AgentProfile, AgentApiError> {
            self.profiles
                .lock()
                .expect("lock")
                .get(&profile_id)
                .cloned()
                .ok_or_else(|| {
                    AgentApiError::not_found(format!("agent profile not found: {profile_id}"))
                })
        }

        async fn start_run(
            &self,
            session_id: &SessionId,
            input: Vec<InputItem>,
            submission_id: SubmissionId,
            notify_on_terminal: Vec<RunTerminalNotifyIntent>,
        ) -> Result<String, AgentApiError> {
            self.started_runs.lock().expect("lock").push(StartedRun {
                session_id: session_id.clone(),
                input,
                submission_id,
                notify_on_terminal,
            });
            Ok("run_1".to_owned())
        }

        async fn enqueue_run(
            &self,
            session_id: &SessionId,
            input: Vec<InputItem>,
            submission_id: SubmissionId,
            notify_on_terminal: Vec<RunTerminalNotifyIntent>,
        ) -> Result<String, AgentApiError> {
            self.started_runs.lock().expect("lock").push(StartedRun {
                session_id: session_id.clone(),
                input,
                submission_id,
                notify_on_terminal,
            });
            Ok("run_1".to_owned())
        }

        async fn deliver_message(
            &self,
            session_id: &SessionId,
            input: Vec<InputItem>,
            submission_id: SubmissionId,
        ) -> Result<(), AgentApiError> {
            self.started_runs.lock().expect("lock").push(StartedRun {
                session_id: session_id.clone(),
                input,
                submission_id,
                notify_on_terminal: Vec::new(),
            });
            Ok(())
        }

        async fn holder_workflow_id(
            &self,
            session_id: &SessionId,
        ) -> Result<String, AgentApiError> {
            Ok(format!("test-universe/{session_id}"))
        }

        async fn read_session(&self, session_id: &SessionId) -> Result<SessionView, AgentApiError> {
            Ok(self
                .sessions
                .lock()
                .expect("lock")
                .get(session_id)
                .cloned()
                .unwrap_or_else(|| {
                    api_session_view(session_id, api::SessionStatus::Idle, Vec::new())
                }))
        }

        async fn read_session_events(
            &self,
            session_id: &SessionId,
            after: Option<u64>,
            limit: u32,
        ) -> Result<SessionEventsReadResponse, AgentApiError> {
            let all_events = self
                .events
                .lock()
                .expect("lock")
                .get(session_id)
                .cloned()
                .unwrap_or_default();
            let events: Vec<_> = all_events
                .iter()
                .filter(|event| after.is_none_or(|after| event.cursor.seq > after))
                .take(limit as usize)
                .cloned()
                .collect();
            Ok(SessionEventsReadResponse {
                next_cursor: events.last().map(|event| event.cursor),
                head_cursor: all_events.last().map(|event| event.cursor),
                events,
                complete: true,
                gap: None,
            })
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_clone_self_creates_child_link_and_starts_run() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = SessionId::new("parent");
        sessions
            .create_session(CreateSession {
                session_id: source.clone(),
                display_name: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        let opening_events =
            core_agent_clone_opening_events(&open_state(), 2).expect("opening events");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: source.clone(),
                expected_head: None,
                events: opening_events,
            })
            .await
            .expect("append open");

        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions.clone(), runtime.clone());
        let output = service
            .spawn(context(source.clone()), spawn_args("summarize"))
            .await
            .expect("spawn")
            .output;

        let child = SessionId::new(output.child_session_id);
        let child_record = sessions
            .load_session(&child)
            .await
            .expect("load")
            .expect("child");
        assert_eq!(child_record.source_session_id, Some(source.clone()));
        assert_eq!(child_record.source_seq, None);

        let links = sessions
            .list_links(ListSessionLinks {
                session_id: source,
                direction: SessionLinkDirection::Outgoing,
                relationship: Some(FLEET_CHILD_RELATIONSHIP.to_owned()),
                limit: 10,
            })
            .await
            .expect("links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].to_session_id, child);

        assert_eq!(
            runtime.started_sessions.lock().expect("lock").as_slice(),
            &[(links[0].to_session_id.clone(), false, None)]
        );
        assert_eq!(output.child_run_id.as_deref(), Some("run_1"));
        assert_eq!(output.status, "created");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_rejects_clone_of_lifecycle_managed_session() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = SessionId::new("managed-parent");
        sessions
            .create_session(CreateSession {
                session_id: source.clone(),
                display_name: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        let universe_id = uuid::Uuid::from_u128(1);
        let admitted = engine::ManagedSessionWorkflowTools::v1(
            Some(engine::WorkflowEndpointRef {
                workflow_id: "controller/managed-parent".to_owned(),
                workflow_kind: "controller.workflow.v1".to_owned(),
            }),
            Vec::new(),
        )
        .admit(universe_id)
        .expect("managed declaration");
        let config = open_state()
            .lifecycle
            .config
            .expect("test state has config");
        let codec = engine::CoreAgentCodec;
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: source.clone(),
                expected_head: None,
                events: vec![
                    codec
                        .encode_uncommitted(&engine::UncommittedCoreAgentEvent {
                            observed_at_ms: 2,
                            joins: engine::CoreAgentJoins::default(),
                            event: engine::CoreAgentEvent::Lifecycle(
                                engine::CoreAgentLifecycleEvent::Opened { config },
                            ),
                        })
                        .expect("encode opened event"),
                    codec
                        .encode_uncommitted(&engine::UncommittedCoreAgentEvent {
                            observed_at_ms: 2,
                            joins: engine::CoreAgentJoins::default(),
                            event: engine::CoreAgentEvent::WorkflowToolConfig(
                                engine::WorkflowToolConfigEvent::ManagedBindingsAdmitted {
                                    session_universe_id: admitted.session_universe_id,
                                    declaration_version: admitted.version,
                                    lifecycle_controller: admitted.lifecycle_controller,
                                    creation_fingerprint: admitted.creation_fingerprint,
                                    bindings: admitted.bindings,
                                },
                            ),
                        })
                        .expect("encode management event"),
                ],
            })
            .await
            .expect("append managed open");

        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions.clone(), runtime.clone());
        let error = service
            .spawn(context(source), spawn_args("summarize"))
            .await
            .expect_err("managed source cannot be cloned");

        assert_eq!(error.kind, api::AgentApiErrorKind::Rejected);
        assert!(error.message.contains("cannot be cloned or forked"));
        assert!(runtime.started_sessions.lock().expect("lock").is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_returns_promise_effect_and_threads_notify_intent() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = open_source_session(&sessions).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        let result = service
            .spawn(context(source), spawn_args("summarize"))
            .await
            .expect("spawn");

        // Output carries a promise id, and a promise-create effect is present
        // for the started run, scoped to the child run.
        let promise_id = result.output.promise.clone().expect("promise id");
        let effect = result.promise_effect.expect("promise effect");
        assert_eq!(effect.kind, engine::PROMISE_CREATE_EFFECT_KIND);
        assert_eq!(effect.data.get("promise_id"), Some(&promise_id));
        assert_eq!(effect.data.get("source"), Some(&"run".to_owned()));

        // The child run carries a notify-intent back to the parent workflow,
        // keyed by the same promise id.
        let started_runs = runtime.started_runs.lock().expect("lock");
        assert_eq!(started_runs.len(), 1);
        let intents = &started_runs[0].notify_on_terminal;
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].token, promise_id);
        assert!(
            intents[0].holder_workflow_id.ends_with("/parent"),
            "holder should be the parent workflow id: {}",
            intents[0].holder_workflow_id
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_close_on_terminal_is_passed_to_child_runtime() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = open_source_session(&sessions).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        service
            .spawn(
                context(source),
                serde_json::from_value(json!({
                    "input": "one-off task",
                    "lifecycle": {
                        "close_on_terminal": true
                    }
                }))
                .expect("spawn args"),
            )
            .await
            .expect("spawn");

        let started = runtime.started_sessions.lock().expect("lock");
        assert_eq!(started.len(), 1);
        assert!(started[0].1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_safe_fork_records_runtime_source_seq() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = open_source_session(sessions.as_ref()).await;
        let expected_seq = sessions
            .safe_fork_seq(&source)
            .await
            .expect("safe fork seq");
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions.clone(), runtime);

        let output = service
            .spawn(
                context(source.clone()),
                serde_json::from_value(json!({
                    "input": "fork work",
                    "base": {
                        "kind": "self",
                        "fork": { "kind": "safe" }
                    }
                }))
                .expect("spawn args"),
            )
            .await
            .expect("spawn")
            .output;

        let child = SessionId::new(output.child_session_id);
        let child_record = sessions
            .load_session(&child)
            .await
            .expect("load")
            .expect("child");
        assert_eq!(child_record.source_session_id, Some(source));
        assert_eq!(child_record.source_seq, Some(expected_seq));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_profile_only_creates_fresh_child_without_source_lineage() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let profile = ProfileSource::Named {
            profile_id: api::ProfileId::new("support"),
        };
        let runtime = Arc::new(FakeRuntime {
            session_store: Some(sessions.clone()),
            ..FakeRuntime::default()
        });
        let service = FleetService::new(sessions.clone(), runtime.clone());

        let output = service
            .spawn(
                context(parent.clone()),
                serde_json::from_value(json!({
                    "input": "support this customer",
                    "base": {
                        "kind": "profile",
                        "profile": {
                            "kind": "named",
                            "profileId": "support"
                        }
                    }
                }))
                .expect("spawn args"),
            )
            .await
            .expect("spawn")
            .output;

        let child = SessionId::new(output.child_session_id);
        let child_record = sessions
            .load_session(&child)
            .await
            .expect("load")
            .expect("child");
        assert_eq!(child_record.source_session_id, None);
        assert_eq!(child_record.source_seq, None);

        assert_eq!(
            runtime.started_sessions.lock().expect("lock").as_slice(),
            &[(child.clone(), false, Some(profile.clone()))]
        );
        assert_eq!(output.child_run_id.as_deref(), Some("run_1"));
        assert_eq!(output.status, "created");

        let links = sessions
            .list_links(ListSessionLinks {
                session_id: parent,
                direction: SessionLinkDirection::Outgoing,
                relationship: Some(FLEET_CHILD_RELATIONSHIP.to_owned()),
                limit: 10,
            })
            .await
            .expect("links");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].to_session_id, child);
        assert_eq!(
            links[0].metadata["profile"],
            serde_json::to_value(profile).expect("profile json")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_profile_base_rejects_vfs_isolate() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);

        let error = service
            .spawn(
                context(parent),
                serde_json::from_value(json!({
                    "input": "bad profile resource policy",
                    "base": {
                        "kind": "profile",
                        "profile": {
                            "kind": "named",
                            "profileId": "support"
                        }
                    },
                    "vfs": "isolate"
                }))
                .expect("spawn args"),
            )
            .await
            .expect_err("profile + vfs isolate must reject");

        assert_eq!(error.kind, api::AgentApiErrorKind::InvalidRequest);
        assert!(error.message.contains("profile requires vfs=share"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_retry_reuses_existing_child() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = SessionId::new("parent");
        sessions
            .create_session(CreateSession {
                session_id: source.clone(),
                display_name: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        let opening_events =
            core_agent_clone_opening_events(&open_state(), 2).expect("opening events");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: source.clone(),
                expected_head: None,
                events: opening_events,
            })
            .await
            .expect("append open");

        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let first = service
            .spawn(context(source.clone()), spawn_args("do work"))
            .await
            .expect("first spawn")
            .output;
        let second = service
            .spawn(context(source), spawn_args("do work"))
            .await
            .expect("retry spawn")
            .output;

        assert_eq!(first.child_session_id, second.child_session_id);
        assert_eq!(second.status, "reused");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn explicit_child_id_collision_without_spawn_metadata_conflicts() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = SessionId::new("parent");
        sessions
            .create_session(CreateSession {
                session_id: source.clone(),
                display_name: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        let opening_events =
            core_agent_clone_opening_events(&open_state(), 2).expect("opening events");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: source.clone(),
                expected_head: None,
                events: opening_events.clone(),
            })
            .await
            .expect("append open");
        let child = SessionId::new("explicit_child");
        sessions
            .create_cloned_session(CreateClonedSession {
                source_session_id: source.clone(),
                session_id: child,
                created_at_ms: 3,
                opening_events,
            })
            .await
            .expect("preexisting clone");

        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let error = service
            .spawn(
                context(source),
                spawn_args_with_child("do work", "explicit_child"),
            )
            .await
            .expect_err("explicit collision must be rejected");

        assert_eq!(error.kind, api::AgentApiErrorKind::Conflict);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn vfs_isolate_rewrites_workspace_links_and_keeps_snapshots() {
        let vfs = Arc::new(TestVfsCatalog::default());
        let sessions = Arc::new(InMemorySessionStore::new());
        let child = SessionId::new("child");
        let source_workspace = VfsWorkspaceId::new("workspace_source");
        let head = BlobRef::from_bytes(b"snapshot-head");
        vfs.create_workspace(CreateVfsWorkspaceRecord {
            workspace_id: source_workspace.clone(),
            display_name: None,
            base_snapshot_ref: None,
            head_snapshot_ref: head.clone(),
            head_totals: ::vfs::VfsTotals::default(),
            created_at_ms: 1,
        })
        .await
        .expect("source workspace");
        let snapshot_ref = BlobRef::from_bytes(b"snapshot-mount");
        let mut state = open_state();
        state.lifecycle.config.as_mut().unwrap().features.vfs = Some(engine::VfsFeature {
            workspace_links: vec![
                WorkspaceLink {
                    path: "/workspace".to_owned(),
                    target: WorkspaceLinkTarget::Workspace {
                        workspace_id: source_workspace.to_string(),
                    },
                    access: WorkspaceLinkAccess::ReadWrite,
                },
                WorkspaceLink {
                    path: "/readonly".to_owned(),
                    target: WorkspaceLinkTarget::Snapshot {
                        snapshot_ref: snapshot_ref.to_string(),
                    },
                    access: WorkspaceLinkAccess::ReadOnly,
                },
            ],
            ..engine::VfsFeature::default()
        });
        sessions
            .create_session(CreateSession {
                session_id: child.clone(),
                display_name: None,
                created_at_ms: 2,
            })
            .await
            .expect("create child");
        let opening_events =
            engine::core_agent_clone_opening_events(&state, 2).expect("opening events");
        sessions
            .append(AppendSessionEvents {
                session_id: child.clone(),
                expected_head: None,
                events: opening_events,
            })
            .await
            .expect("open child");

        let service = FleetService::new(sessions.clone(), Arc::new(FakeRuntime::default()))
            .with_vfs_stores(vfs.clone());
        service
            .apply_resource_policies(
                &child,
                &child,
                10,
                &serde_json::from_value(json!({
                    "input": "do work",
                    "vfs": "isolate"
                }))
                .expect("args"),
            )
            .await
            .expect("isolate");

        let entries = api_projection::read_all_session_entries(sessions.as_ref(), &child, 100)
            .await
            .expect("child entries");
        let state = api_projection::replay_core_agent_state(&entries).expect("child state");
        let links = &state
            .lifecycle
            .config
            .unwrap()
            .features
            .vfs
            .unwrap()
            .workspace_links;
        let WorkspaceLinkTarget::Workspace { workspace_id } = &links[0].target else {
            panic!("workspace link target");
        };
        let workspace_id = VfsWorkspaceId::try_new(workspace_id.clone()).unwrap();
        assert_ne!(workspace_id, source_workspace);
        let child_workspace = vfs
            .read_workspace(&workspace_id)
            .await
            .expect("child workspace");
        assert_eq!(child_workspace.base_snapshot_ref, Some(head.clone()));
        assert_eq!(child_workspace.head_snapshot_ref, head);
        assert_eq!(
            links[1].target,
            WorkspaceLinkTarget::Snapshot {
                snapshot_ref: snapshot_ref.to_string()
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fleet_executor_runs_spawn_and_writes_output_blobs() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let source = open_source_session(sessions.as_ref()).await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);
        let arguments_ref = blobs
            .put_bytes(br#"{"input":"do work"}"#.to_vec())
            .await
            .expect("args");
        let result = executor
            .invoke(
                context(source),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_1"),
                    tool_name: ToolName::new(AGENT_SPAWN_TOOL_NAME),
                    arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        let output_ref = result.output_ref.as_ref().expect("output");
        let output: AgentSpawnOutput =
            serde_json::from_slice(&blobs.read_bytes(output_ref).await.expect("read output"))
                .expect("decode output");
        assert!(output.child_session_id.starts_with("agent_"));
        let visible_ref = visible_tool_result_ref(&result);
        let visible = blobs.read_text(&visible_ref).await.expect("read visible");
        assert!(visible.contains("started run"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_to_linked_child_delivers_envelope_with_deterministic_submission_id() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = create_linked_child(sessions.as_ref(), &parent).await;
        let runtime = Arc::new(FakeRuntime::default());
        let counting_sessions = Arc::new(CountingSessionStore::new(sessions));
        let service = FleetService::new(counting_sessions.clone(), runtime.clone());

        let output = service
            .send(
                context(parent.clone()),
                serde_json::from_value(json!({
                    "to": { "kind": "session", "target_session_id": child.as_str() },
                    "text": "do more work",
                    "payload": { "answer": 42 },
                    "input": [
                        { "type": "text", "text": "trailing context" }
                    ]
                }))
                .expect("send args"),
            )
            .await
            .expect("send")
            .output;

        assert_eq!(output.target_session_id, child.as_str());
        assert!(output.submission_id.starts_with("fleet_send_"));
        assert!(runtime.started_sessions.lock().expect("lock").is_empty());
        assert_eq!(counting_sessions.event_reads(), 0);
        let started_runs = runtime.started_runs.lock().expect("lock");
        assert_eq!(started_runs.len(), 1);
        assert_eq!(started_runs[0].session_id, child);
        assert_eq!(started_runs[0].input.len(), 2);
        let envelope = text_item_json(&started_runs[0].input[0]);
        assert_eq!(envelope["fleet_send"]["from_session_id"], "parent");
        assert!(envelope["fleet_send"].get("kind").is_none());
        assert_eq!(envelope["fleet_send"]["payload"], json!({ "answer": 42 }));
        assert_eq!(envelope["text"], "do more work");
        assert_eq!(
            started_runs[0].input[1],
            InputItem::Text {
                text: "trailing context".to_owned()
            }
        );
        assert!(
            started_runs[0]
                .submission_id
                .as_str()
                .starts_with("fleet_send_"),
            "submission id should be Fleet-derived, got {}",
            started_runs[0].submission_id
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_returns_promise_effect_and_notify_intent() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = create_linked_child(sessions.as_ref(), &parent).await;
        let runtime = Arc::new(FakeRuntime::default());
        let counting_sessions = Arc::new(CountingSessionStore::new(sessions));
        let service = FleetService::new(counting_sessions.clone(), runtime.clone());

        let result = service
            .request(
                context(parent.clone()),
                serde_json::from_value(json!({
                    "to": { "kind": "session", "target_session_id": child.as_str() },
                    "text": "do more work"
                }))
                .expect("request args"),
            )
            .await
            .expect("request");

        let promise = result.output.promise.clone();
        let effect = result.promise_effect.expect("promise effect");
        assert_eq!(effect.kind, engine::PROMISE_CREATE_EFFECT_KIND);
        assert_eq!(effect.data.get("source"), Some(&"run".to_owned()));
        assert_eq!(effect.data.get("promise_id"), Some(&promise));

        let started_runs = runtime.started_runs.lock().expect("lock");
        assert_eq!(started_runs.len(), 1);
        assert!(
            started_runs[0]
                .submission_id
                .as_str()
                .starts_with("fleet_request_")
        );
        assert_eq!(started_runs[0].notify_on_terminal.len(), 1);
        let intent = &started_runs[0].notify_on_terminal[0];
        assert_eq!(intent.token, promise);
        let envelope = text_item_json(&started_runs[0].input[0]);
        assert_eq!(envelope["fleet_request"]["from_session_id"], "parent");
        assert_eq!(envelope["text"], "do more work");
        assert_eq!(counting_sessions.event_reads(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_to_parent_uses_the_same_linked_target_admission() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = create_linked_child(sessions.as_ref(), &parent).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        let result = service
            .request(
                context(child),
                serde_json::from_value(json!({
                    "to": { "kind": "parent" },
                    "text": "please do work"
                }))
                .expect("request args"),
            )
            .await
            .expect("request parent");

        assert_eq!(result.output.target_session_id, parent.as_str());
        let started_runs = runtime.started_runs.lock().expect("lock");
        assert_eq!(started_runs.len(), 1);
        assert_eq!(started_runs[0].session_id, parent);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_to_parent_resolves_incoming_spawn_link() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = create_linked_child(sessions.as_ref(), &parent).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        let output = service
            .send(
                context(child),
                serde_json::from_value(json!({
                    "to": { "kind": "parent" },
                    "text": "done"
                }))
                .expect("send args"),
            )
            .await
            .expect("send")
            .output;

        assert_eq!(output.target_session_id, parent.as_str());
        assert!(output.submission_id.starts_with("fleet_send_"));
        let started_runs = runtime.started_runs.lock().expect("lock");
        assert_eq!(started_runs[0].session_id, parent);
        let envelope = text_item_json(&started_runs[0].input[0]);
        assert!(envelope["fleet_send"].get("kind").is_none());
        assert_eq!(envelope["text"], "done");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn detach_promises_returns_detach_effects() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let blobs = Arc::new(InMemoryBlobStore::new());
        let arguments_ref = blobs
            .put_bytes(br#"{"promises":["promise_request_1"]}"#.to_vec())
            .await
            .expect("detach args");
        let executor = FleetToolExecutor::new(blobs, service);
        let result = executor
            .invoke(
                context(parent),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_detach"),
                    tool_name: ToolName::new(DETACH_TOOL_NAME),
                    arguments_ref,
                    workflow_tool: None,
                    promise_control: Some(engine::PromiseControlCallRuntime::v1(vec![
                        engine::PromiseControlRuntime {
                            promise_id: PromiseId::new("promise_request_1"),
                            state: engine::PromiseControlStateRuntime::Known {
                                ownership: engine::PromiseOwnership::Model,
                                scope: engine::PromiseScope::Run {
                                    run_id: RunId::new(1),
                                },
                                promise_status: engine::PromiseStatus::Pending,
                            },
                        },
                    ])),
                },
            )
            .await
            .expect("detach");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.effects[0].kind, engine::PROMISE_DETACH_EFFECT_KIND);
        assert_eq!(
            result.effects[0].data.get("promise_id"),
            Some(&"promise_request_1".to_owned())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_without_link_is_rejected() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        let error = service
            .send(
                context(parent),
                serde_json::from_value(json!({
                    "to": { "kind": "session", "target_session_id": "other" },
                    "text": "hello"
                }))
                .expect("send args"),
            )
            .await
            .expect_err("unlinked send must fail");

        assert!(error.to_string().contains("not directly linked"));
        assert!(runtime.started_runs.lock().expect("lock").is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn send_to_parent_from_root_is_rejected() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());

        let error = service
            .send(
                context(parent),
                serde_json::from_value(json!({
                    "to": { "kind": "parent" },
                    "text": "hello"
                }))
                .expect("send args"),
            )
            .await
            .expect_err("root has no parent");

        assert!(error.to_string().contains("no Fleet parent"));
        assert!(runtime.started_runs.lock().expect("lock").is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_run_input_does_not_inject_report_back_instruction() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions.clone(), runtime.clone());

        let output = service
            .spawn(
                context(parent.clone()),
                serde_json::from_value(json!({
                    "input": "do work"
                }))
                .expect("spawn args"),
            )
            .await
            .expect("spawn")
            .output;

        let child = SessionId::new(output.child_session_id);
        let child_record = sessions
            .load_session(&child)
            .await
            .expect("load")
            .expect("child");
        assert_eq!(child_record.source_session_id, Some(parent));
        let started_runs = runtime.started_runs.lock().expect("lock");
        let text = text_item(&started_runs[0].input[0]);
        assert_eq!(text, "do work");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_returns_linked_children_with_compact_status() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = SessionId::new("child");
        sessions
            .create_cloned_session(CreateClonedSession {
                source_session_id: parent.clone(),
                session_id: child.clone(),
                created_at_ms: 20,
                opening_events: core_agent_clone_opening_events(&open_state(), 20)
                    .expect("opening events"),
            })
            .await
            .expect("child");
        sessions
            .upsert_link(UpsertSessionLink {
                from_session_id: parent.clone(),
                to_session_id: child.clone(),
                relationship: FLEET_CHILD_RELATIONSHIP.to_owned(),
                created_at_ms: 21,
                metadata: json!({ "kind": "fleet_spawn" }),
            })
            .await
            .expect("link");
        let runtime = Arc::new(FakeRuntime::default());
        runtime.sessions.lock().expect("lock").insert(
            child.clone(),
            api_session_view(
                &child,
                api::SessionStatus::Active,
                vec![api_run_view("run_7", ApiRunStatus::Running)],
            ),
        );
        let service = FleetService::new(sessions, runtime);

        let output = service
            .list(
                context(parent.clone()),
                AgentListArgs {
                    target_session_id: None,
                    direction: AgentListDirection::Children,
                    limit: Some(10),
                },
            )
            .await
            .expect("list");

        assert_eq!(output.target_session_id, "parent");
        assert_eq!(output.agents.len(), 1);
        let agent = &output.agents[0];
        assert_eq!(agent.session_id, "child");
        assert_eq!(agent.relationship, FLEET_CHILD_RELATIONSHIP);
        assert_eq!(agent.status.as_deref(), Some("active"));
        assert_eq!(agent.active_run_id.as_deref(), Some("run_7"));
        assert_eq!(
            agent.lineage.source_session_id.as_deref(),
            Some(parent.as_str())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn read_returns_session_lineage_resources_links_and_recent_activity() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = SessionId::new("child");
        sessions
            .create_cloned_session(CreateClonedSession {
                source_session_id: parent.clone(),
                session_id: child.clone(),
                created_at_ms: 20,
                opening_events: core_agent_clone_opening_events(&open_state(), 20)
                    .expect("opening events"),
            })
            .await
            .expect("child");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: child.clone(),
                expected_head: sessions.head(&child).await.expect("child head"),
                events: vec![
                    stored_test_event(30, "lightspeed.test.1"),
                    stored_test_event(31, "lightspeed.test.2"),
                    stored_test_event(32, "lightspeed.test.3"),
                ],
            })
            .await
            .expect("append child events");
        sessions
            .upsert_link(UpsertSessionLink {
                from_session_id: parent,
                to_session_id: child.clone(),
                relationship: FLEET_CHILD_RELATIONSHIP.to_owned(),
                created_at_ms: 21,
                metadata: json!({ "kind": "fleet_spawn" }),
            })
            .await
            .expect("link");
        let runtime = Arc::new(FakeRuntime::default());
        runtime.sessions.lock().expect("lock").insert(
            child.clone(),
            api_session_view(&child, api::SessionStatus::Idle, Vec::new()),
        );
        runtime.events.lock().expect("lock").insert(
            child.clone(),
            vec![
                api_event(&child, 1),
                api_event(&child, 2),
                api_event(&child, 3),
            ],
        );
        runtime
            .sessions
            .lock()
            .expect("lock")
            .get_mut(&child)
            .expect("child session")
            .active_environment_id = Some("env_1".to_owned());
        let service = FleetService::new(sessions, runtime);

        let output = service
            .read(AgentReadArgs {
                target_session_id: child.as_str().to_owned(),
                recent_transcript: Some(tools::fleet::RecentTranscriptSelector {
                    turns: Some(1),
                    events: None,
                }),
                recent_events: Some(tools::fleet::RecentEventsSelector { limit: 3 }),
            })
            .await
            .expect("read");

        assert_eq!(output.session_id, "child");
        assert_eq!(output.session["id"], "child");
        assert_eq!(
            output.session["config"]["features"]["fleet"]["version"],
            api::CURRENT_FEATURE_VERSION
        );
        assert_eq!(output.lineage.source_session_id.as_deref(), Some("parent"));
        assert_eq!(output.links.len(), 1);
        assert_eq!(output.environments["activeEnvironmentId"], "env_1");
        assert_eq!(output.recent_events.len(), 2);
        assert_eq!(output.recent_events[0]["cursor"]["seq"], 2);
        assert_eq!(output.recent_transcript.len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn await_parser_satisfied_promise_still_defers() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        append_parent_with_promise(
            sessions.as_ref(),
            &parent,
            "promise_done",
            engine::PromiseStatus::Resolved,
        )
        .await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);

        let spec = service
            .await_promises(
                &context(parent),
                ToolCallId::new("call_await"),
                serde_json::from_value(json!({
                    "promises": ["promise_done"],
                    "mode": "all"
                }))
                .expect("await args"),
            )
            .await
            .expect("await");

        assert_eq!(spec.mode, engine::AwaitMode::All);
        assert_eq!(spec.promise_ids, vec![PromiseId::new("promise_done")]);
        assert_eq!(spec.deadline_at_ms, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn await_parser_sets_absolute_deadline() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        append_parent_with_promise(
            sessions.as_ref(),
            &parent,
            "promise_pending",
            engine::PromiseStatus::Pending,
        )
        .await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);

        let mut ctx = context(parent);
        ctx.observed_at_ms = 10_000;
        let spec = service
            .await_promises(
                &ctx,
                ToolCallId::new("call_await"),
                serde_json::from_value(json!({
                    "promises": ["promise_pending"],
                    "mode": "all",
                    "timeout_ms": 5000
                }))
                .expect("await args"),
            )
            .await
            .expect("await");

        assert_eq!(spec.mode, engine::AwaitMode::All);
        assert_eq!(spec.promise_ids, vec![PromiseId::new("promise_pending")]);
        assert_eq!(spec.deadline_at_ms, Some(15_000));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn await_parser_zero_timeout_defers_with_immediate_deadline() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        append_parent_with_promise(
            sessions.as_ref(),
            &parent,
            "promise_pending",
            engine::PromiseStatus::Pending,
        )
        .await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);

        let spec = service
            .await_promises(
                &context(parent),
                ToolCallId::new("call_await"),
                serde_json::from_value(json!({
                    "promises": ["promise_pending"],
                    "mode": "all",
                    "timeout_ms": 0
                }))
                .expect("await args"),
            )
            .await
            .expect("await");
        assert_eq!(spec.promise_ids, vec![PromiseId::new("promise_pending")]);
        assert_eq!(spec.deadline_at_ms, Some(10));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn await_parser_unknown_promise_defers_for_engine_validation() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);

        let spec = service
            .await_promises(
                &context(parent),
                ToolCallId::new("call_await"),
                serde_json::from_value(json!({
                    "promises": ["promise_missing"],
                    "mode": "all"
                }))
                .expect("await args"),
            )
            .await
            .expect("await");
        assert_eq!(spec.promise_ids, vec![PromiseId::new("promise_missing")]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fleet_executor_lone_await_returns_deferred_batch() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        append_parent_with_promise(
            sessions.as_ref(),
            &parent,
            "promise_pending",
            engine::PromiseStatus::Pending,
        )
        .await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);
        let arguments_ref = blobs
            .put_bytes(
                r#"{"promises":["promise_pending"],"timeout_ms":5000}"#
                    .as_bytes()
                    .to_vec(),
            )
            .await
            .expect("args");

        let outcome = executor
            .invoke_await_batch(
                context(parent),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_await"),
                    tool_name: ToolName::new(AWAIT_TOOL_NAME),
                    arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke");

        let ToolBatchOutcome::Deferred {
            batch_id,
            call_id,
            completed_results: _,
            spec,
        } = outcome
        else {
            panic!("await should defer");
        };
        assert_eq!(batch_id, ToolBatchId::new(1));
        assert_eq!(call_id, ToolCallId::new("call_await"));
        assert_eq!(spec.promise_ids, vec![PromiseId::new("promise_pending")]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fleet_executor_runs_read_and_writes_output_blobs() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let child = open_source_session(sessions.as_ref()).await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let runtime = Arc::new(FakeRuntime::default());
        let mut run = api_run_view("run_1", ApiRunStatus::Completed);
        run.entries.push(api::ContextEntryView {
            id: "item_1".to_owned(),
            key: None,
            kind: api::ContextEntryKindView::ToolResult {
                call_id: "call_shell".to_owned(),
                is_error: false,
            },
            content_ref: "sha256:tool".to_owned(),
            media_type: None,
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
            text: Some("/opt\n## main...origin/main".to_owned()),
            display: None,
        });
        run.entries.push(api::ContextEntryView {
            id: "item_2".to_owned(),
            key: None,
            kind: api::ContextEntryKindView::Message {
                role: api::ContextMessageRoleView::Assistant,
            },
            content_ref: "sha256:assistant".to_owned(),
            media_type: None,
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
            text: Some("Command completed.".to_owned()),
            display: None,
        });
        runtime.sessions.lock().expect("lock").insert(
            child.clone(),
            api_session_view(&child, api::SessionStatus::Idle, vec![run]),
        );
        let service = FleetService::new(sessions, runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);
        let arguments_ref = blobs
            .put_bytes(br#"{"target_session_id":"parent"}"#.to_vec())
            .await
            .expect("args");

        let result = executor
            .invoke(
                context(child),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_1"),
                    tool_name: ToolName::new(AGENT_READ_TOOL_NAME),
                    arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        let output_ref = result.output_ref.as_ref().expect("output");
        let output: AgentReadOutput =
            serde_json::from_slice(&blobs.read_bytes(output_ref).await.expect("read output"))
                .expect("decode output");
        assert_eq!(output.session_id, "parent");
        let visible_ref = visible_tool_result_ref(&result);
        assert_eq!(&visible_ref, output_ref);
        assert_eq!(result.model_visible_context_entries.len(), 1);
        assert!(matches!(
            result.model_visible_context_entries[0].kind,
            ContextEntryKind::ToolResult { .. }
        ));
        assert_eq!(output.session["runs"][0]["id"], "run_1");
        assert_eq!(output.session["runs"][0]["status"], "completed");
        assert_eq!(
            output.session["runs"][0]["entries"][0]["text"],
            "/opt\n## main...origin/main"
        );
        assert_eq!(
            output.session["runs"][0]["entries"][1]["text"],
            "Command completed."
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fleet_executor_runs_profile_tools_and_writes_output_blobs() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let profile = test_profile("support");
        let runtime = Arc::new(FakeRuntime::default());
        runtime
            .profiles
            .lock()
            .expect("lock")
            .insert(profile.profile_id.clone(), profile.clone());
        let service = FleetService::new(sessions, runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);

        let list_arguments_ref = blobs.put_bytes(br#"{}"#.to_vec()).await.expect("args");
        let list_result = executor
            .invoke(
                context(parent.clone()),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_profile_list"),
                    tool_name: ToolName::new(PROFILE_LIST_TOOL_NAME),
                    arguments_ref: list_arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke list");

        assert_eq!(list_result.status, ToolCallStatus::Succeeded);
        let list_output_ref = list_result.output_ref.as_ref().expect("list output");
        let list_output: ProfileListOutput = serde_json::from_slice(
            &blobs
                .read_bytes(list_output_ref)
                .await
                .expect("read list output"),
        )
        .expect("decode list output");
        assert_eq!(list_output.profiles, vec![profile.summary()]);
        let list_visible_ref = visible_tool_result_ref(&list_result);
        let list_visible = blobs
            .read_text(&list_visible_ref)
            .await
            .expect("read list visible");
        assert!(list_visible.contains("Found 1 agent profile"));

        let read_arguments_ref = blobs
            .put_bytes(br#"{"profile_id":"support"}"#.to_vec())
            .await
            .expect("args");
        let read_result = executor
            .invoke(
                context(parent),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_profile_read"),
                    tool_name: ToolName::new(PROFILE_READ_TOOL_NAME),
                    arguments_ref: read_arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke read");

        assert_eq!(read_result.status, ToolCallStatus::Succeeded);
        let read_output_ref = read_result.output_ref.as_ref().expect("read output");
        let read_output: ProfileReadOutput = serde_json::from_slice(
            &blobs
                .read_bytes(read_output_ref)
                .await
                .expect("read profile output"),
        )
        .expect("decode read output");
        assert_eq!(read_output.profile, profile);
        let read_visible_ref = visible_tool_result_ref(&read_result);
        let read_visible = blobs
            .read_text(&read_visible_ref)
            .await
            .expect("read visible");
        assert!(read_visible.contains("Read profile support revision 1"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn profile_tools_apply_fleet_profile_policy() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let fleet_policy = FleetFeature {
            profiles: FleetProfilesConfig {
                allow: Some(vec!["support".to_owned(), "admin".to_owned()]),
                deny: vec!["admin".to_owned()],
                inline: true,
            },
            spawn: FleetSpawnConfig::default(),
            ..FleetFeature::default()
        };
        let parent =
            open_source_session_with_fleet_config(sessions.as_ref(), fleet_policy.clone()).await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let support = test_profile("support");
        let admin = test_profile("admin");
        let hidden = test_profile("hidden");
        let runtime = Arc::new(FakeRuntime::default());
        {
            let mut profiles = runtime.profiles.lock().expect("lock");
            profiles.insert(support.profile_id.clone(), support.clone());
            profiles.insert(admin.profile_id.clone(), admin);
            profiles.insert(hidden.profile_id.clone(), hidden);
        }
        let counting_sessions = Arc::new(CountingSessionStore::new(sessions));
        let service = FleetService::new(counting_sessions.clone(), runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);
        let mut invocation_context = context(parent.clone());
        invocation_context.fleet_policy = Some(fleet_policy);

        let list_result = executor
            .invoke(
                invocation_context.clone(),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_profile_list"),
                    tool_name: ToolName::new(PROFILE_LIST_TOOL_NAME),
                    arguments_ref: blobs.put_bytes(br#"{}"#.to_vec()).await.expect("args"),
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke list");
        assert_eq!(list_result.status, ToolCallStatus::Succeeded);
        let list_output: ProfileListOutput = serde_json::from_slice(
            &blobs
                .read_bytes(list_result.output_ref.as_ref().expect("list output"))
                .await
                .expect("read list output"),
        )
        .expect("decode list output");
        assert_eq!(list_output.profiles, vec![support.summary()]);

        let denied_read = executor
            .invoke(
                invocation_context,
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_profile_read"),
                    tool_name: ToolName::new(PROFILE_READ_TOOL_NAME),
                    arguments_ref: blobs
                        .put_bytes(br#"{"profile_id":"admin"}"#.to_vec())
                        .await
                        .expect("args"),
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke read");
        assert_eq!(denied_read.status, ToolCallStatus::Failed);
        let error = blobs
            .read_text(denied_read.error_ref.as_ref().expect("error"))
            .await
            .expect("read error");
        assert!(error.contains("profile admin is not allowed"));
        assert_eq!(counting_sessions.event_reads(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_rejects_disallowed_profile_sources_and_bases() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let fleet_policy = FleetFeature {
            profiles: FleetProfilesConfig {
                allow: Some(vec!["worker".to_owned()]),
                deny: Vec::new(),
                inline: false,
            },
            spawn: FleetSpawnConfig {
                bases: Some(vec![FleetSpawnBase::Profile]),
            },
            ..FleetFeature::default()
        };
        let parent =
            open_source_session_with_fleet_config(sessions.as_ref(), fleet_policy.clone()).await;
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime.clone());
        let spawn_context = |parent: SessionId| {
            let mut context = context(parent);
            context.fleet_policy = Some(fleet_policy.clone());
            context
        };

        let mut self_args = spawn_args("clone yourself");
        self_args.base = AgentSpawnBase::Self_ { fork: None };
        let self_error = service
            .spawn(spawn_context(parent.clone()), self_args)
            .await
            .expect_err("self base should be rejected");
        assert!(self_error.message.contains("base self is not allowed"));

        let mut denied_named = spawn_args("spawn admin");
        denied_named.base = AgentSpawnBase::Profile {
            profile: ProfileSource::Named {
                profile_id: ProfileId::new("admin"),
            },
        };
        let named_error = service
            .spawn(spawn_context(parent.clone()), denied_named)
            .await
            .expect_err("named profile should be rejected");
        assert!(named_error.message.contains("profile admin is not allowed"));

        let mut inline = spawn_args("spawn inline");
        inline.base = AgentSpawnBase::Profile {
            profile: ProfileSource::Inline {
                profile: Box::new(api::InlineAgentProfile {
                    display_name: None,
                    description: None,
                    document: api::ProfileDocument::default(),
                }),
            },
        };
        let inline_error = service
            .spawn(spawn_context(parent), inline)
            .await
            .expect_err("inline profile should be rejected");
        assert!(
            inline_error
                .message
                .contains("inline profiles are not allowed")
        );
        assert!(runtime.started_sessions.lock().expect("lock").is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fleet_executor_runs_send_and_writes_output_blobs() {
        let sessions = Arc::new(InMemorySessionStore::new());
        let parent = open_source_session(sessions.as_ref()).await;
        let child = create_linked_child(sessions.as_ref(), &parent).await;
        let blobs = Arc::new(engine::storage::InMemoryBlobStore::new());
        let runtime = Arc::new(FakeRuntime::default());
        let service = FleetService::new(sessions, runtime);
        let executor = FleetToolExecutor::new(blobs.clone(), service);
        let arguments_ref = blobs
            .put_bytes(
                format!(
                    r#"{{"to":{{"kind":"session","target_session_id":"{}"}},"text":"do more work"}}"#,
                    child.as_str()
                )
                .into_bytes(),
            )
            .await
            .expect("args");

        let result = executor
            .invoke(
                context(parent),
                &ToolInvocationRequest {
                    call_id: ToolCallId::new("call_1"),
                    tool_name: ToolName::new(AGENT_SEND_TOOL_NAME),
                    arguments_ref,
                    workflow_tool: None,
                    promise_control: None,
                },
            )
            .await
            .expect("invoke");

        assert_eq!(result.status, ToolCallStatus::Succeeded);
        let output_ref = result.output_ref.as_ref().expect("output");
        let output: AgentSendOutput =
            serde_json::from_slice(&blobs.read_bytes(output_ref).await.expect("read output"))
                .expect("decode output");
        assert_eq!(output.target_session_id, child.as_str());
        let visible_ref = visible_tool_result_ref(&result);
        let visible = blobs.read_text(&visible_ref).await.expect("read visible");
        assert!(visible.contains("Delivered message"));
    }

    fn spawn_args(input: &str) -> AgentSpawnArgs {
        serde_json::from_value(json!({ "input": input })).expect("args")
    }

    fn spawn_args_with_child(input: &str, child_session_id: &str) -> AgentSpawnArgs {
        serde_json::from_value(json!({
            "input": input,
            "child_session_id": child_session_id
        }))
        .expect("args")
    }

    fn test_profile(profile_id: &str) -> AgentProfile {
        AgentProfile {
            profile_id: ProfileId::new(profile_id),
            display_name: Some("Support".to_owned()),
            description: Some("Ticket support profile".to_owned()),
            revision: 1,
            document: api::ProfileDocument {
                instructions: Some(api::ProfileInstructions::Text {
                    text: "Be concise.".to_owned(),
                }),
                ..api::ProfileDocument::default()
            },
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    async fn open_source_session(sessions: &InMemorySessionStore) -> SessionId {
        open_source_session_with_fleet_config(sessions, FleetFeature::default()).await
    }

    async fn open_source_session_with_fleet_config(
        sessions: &InMemorySessionStore,
        fleet: FleetFeature,
    ) -> SessionId {
        let source = SessionId::new("parent");
        sessions
            .create_session(CreateSession {
                session_id: source.clone(),
                display_name: None,
                created_at_ms: 1,
            })
            .await
            .expect("create source");
        let opening_events =
            core_agent_clone_opening_events(&open_state_with_fleet_config(fleet), 2)
                .expect("opening events");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: source.clone(),
                expected_head: None,
                events: opening_events,
            })
            .await
            .expect("append open");
        source
    }

    async fn create_linked_child(sessions: &InMemorySessionStore, parent: &SessionId) -> SessionId {
        let child = SessionId::new("child");
        sessions
            .create_cloned_session(CreateClonedSession {
                source_session_id: parent.clone(),
                session_id: child.clone(),
                created_at_ms: 20,
                opening_events: core_agent_clone_opening_events(&open_state(), 20)
                    .expect("opening events"),
            })
            .await
            .expect("child");
        sessions
            .upsert_link(UpsertSessionLink {
                from_session_id: parent.clone(),
                to_session_id: child.clone(),
                relationship: FLEET_CHILD_RELATIONSHIP.to_owned(),
                created_at_ms: 21,
                metadata: json!({ "kind": "fleet_spawn" }),
            })
            .await
            .expect("link");
        child
    }

    /// Append an active run (run 1, matching the test `context()`), then a
    /// `Promise(Created)` in the given status scoped to that run. The parent
    /// session must already be open (via `open_source_session`).
    async fn append_parent_with_promise(
        sessions: &InMemorySessionStore,
        session_id: &SessionId,
        promise_id: &str,
        status: engine::PromiseStatus,
    ) {
        let run_id = RunId::new(1);
        let mut events = vec![
            core_uncommitted_event(
                40,
                engine::CoreAgentEvent::Run(engine::RunEvent::Accepted(engine::AcceptedRunEvent {
                    notify_on_terminal: Vec::new(),
                    run_id,
                    submission_id: None,
                    origin: engine::RunOrigin::Requested,
                    source: engine::RunSource::Input { input: Vec::new() },
                    run_config: RunConfig::default(),
                    config_revision: 0,
                })),
            ),
            core_uncommitted_event(
                41,
                engine::CoreAgentEvent::Run(engine::RunEvent::Started { run_id }),
            ),
            core_uncommitted_event(
                42,
                engine::CoreAgentEvent::Promise(engine::PromiseEvent::Created {
                    promise: engine::Promise {
                        promise_id: engine::PromiseId::new(promise_id),
                        source: engine::PromiseSource::Run {
                            target_session_id: "child".to_owned(),
                            target_run_id: 2,
                        },
                        scope: engine::PromiseScope::Run { run_id },
                        ownership: engine::PromiseOwnership::Model,
                        status: engine::PromiseStatus::Pending,
                        payload_ref: None,
                        error_ref: None,
                        deadline_ms: None,
                    },
                }),
            ),
        ];
        if status.is_terminal() {
            let resolution_event = match status {
                engine::PromiseStatus::Resolved => engine::PromiseEvent::Resolved {
                    promise_id: engine::PromiseId::new(promise_id),
                    payload_ref: None,
                },
                engine::PromiseStatus::Failed => engine::PromiseEvent::Failed {
                    promise_id: engine::PromiseId::new(promise_id),
                    error_ref: None,
                },
                engine::PromiseStatus::Cancelled => engine::PromiseEvent::Cancelled {
                    promise_id: engine::PromiseId::new(promise_id),
                },
                engine::PromiseStatus::Pending => unreachable!(),
            };
            events.push(core_uncommitted_event(
                43,
                engine::CoreAgentEvent::Promise(resolution_event),
            ));
        }
        let head = sessions.head(session_id).await.expect("head");
        sessions
            .append(engine::storage::AppendSessionEvents {
                session_id: session_id.clone(),
                expected_head: head,
                events,
            })
            .await
            .expect("append promise");
    }

    fn core_uncommitted_event(
        observed_at_ms: u64,
        event: engine::CoreAgentEvent,
    ) -> engine::storage::UncommittedStoredEvent {
        engine::CoreAgentCodec
            .encode_uncommitted(&engine::UncommittedCoreAgentEvent {
                observed_at_ms,
                joins: Default::default(),
                event,
            })
            .expect("encode core event")
    }

    fn text_item(item: &InputItem) -> &str {
        let InputItem::Text { text } = item else {
            panic!("expected text input item");
        };
        text
    }

    fn text_item_json(item: &InputItem) -> Value {
        serde_json::from_str(text_item(item)).expect("input text should be JSON")
    }

    fn context(parent_session_id: SessionId) -> FleetInvocationContext {
        FleetInvocationContext {
            parent_session_id,
            parent_run_id: RunId::new(1),
            turn_id: TurnId::new(1),
            batch_id: ToolBatchId::new(1),
            call_id: ToolCallId::new("call_1"),
            observed_at_ms: 10,
            fleet_policy: Some(FleetFeature::default()),
        }
    }

    fn open_state() -> engine::CoreAgentState {
        open_state_with_fleet_config(FleetFeature::default())
    }

    fn open_state_with_fleet_config(fleet: FleetFeature) -> engine::CoreAgentState {
        let mut state = engine::CoreAgentState::new();
        state.lifecycle.config = Some(SessionConfig {
            model: ModelSelection {
                api_kind: ProviderApiKind::OpenAiResponses,
                provider_id: "test".to_owned(),
                model: "test-model".to_owned(),
            },
            generation: GenerationConfig::default(),
            limits: LimitsConfig::default(),
            context: ContextConfig { compaction: None },
            features: FeaturesConfig {
                fleet: Some(fleet),
                ..FeaturesConfig::default()
            },
        });
        state
    }

    fn api_session_view(
        session_id: &SessionId,
        status: api::SessionStatus,
        runs: Vec<api::RunView>,
    ) -> SessionView {
        SessionView {
            id: session_id.as_str().to_owned(),
            status,
            display_name: None,
            managed: false,
            config_revision: 1,
            config: Some(api::SessionConfig {
                model: Some(api::ModelConfig {
                    provider_id: "test".to_owned(),
                    api_kind: "openaiResponses".to_owned(),
                    model: "test-model".to_owned(),
                }),
                generation: None,
                limits: None,
                context: None,
                features: Some(api::FeaturesConfig {
                    vfs: Some(api::VfsFeature {
                        version: api::CURRENT_FEATURE_VERSION,
                        workspace_links: Vec::new(),
                        tools: Some(api::VfsToolSurface::Edit),
                        prompts: None,
                        skills: None,
                    }),
                    fleet: Some(api::FleetFeature {
                        version: api::CURRENT_FEATURE_VERSION,
                        profiles: None,
                        spawn: None,
                    }),
                    ..api::FeaturesConfig::default()
                }),
            }),
            active_environment_id: None,
            created_at_ms: 1,
            updated_at_ms: 2,
            runs,
            active_context: api::ContextView::default(),
            active_tools: api::ActiveToolsView::default(),
            management: None,
        }
    }

    fn api_run_view(run_id: &str, status: ApiRunStatus) -> api::RunView {
        api::RunView {
            id: run_id.to_owned(),
            status,
            source: api::RunViewSource::Input { items: Vec::new() },
            entries: Vec::new(),
            tool_batches: Vec::new(),
        }
    }

    fn api_event(session_id: &SessionId, seq: u64) -> api::SessionEventView {
        api::SessionEventView {
            cursor: api::EventCursor { seq },
            session_id: session_id.as_str().to_owned(),
            observed_at_ms: seq,
            joins: api::EventJoinsView::default(),
            kind: api::SessionEventKindView::SessionConfigChanged {
                model: None,
                revision: seq,
            },
        }
    }

    fn stored_test_event(
        at_ms: u64,
        kind: &'static str,
    ) -> engine::storage::UncommittedStoredEvent {
        engine::storage::UncommittedStoredEvent {
            observed_at_ms: at_ms,
            joins: Default::default(),
            event: engine::StoredEvent::new(kind, 1, Value::Object(Default::default())),
        }
    }

    #[derive(Default)]
    struct TestVfsCatalog {
        workspaces: Mutex<BTreeMap<VfsWorkspaceId, VfsWorkspaceRecord>>,
    }

    #[async_trait]
    impl VfsWorkspaceStore for TestVfsCatalog {
        async fn create_workspace(
            &self,
            record: CreateVfsWorkspaceRecord,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            let mut workspaces = self.workspaces.lock().expect("workspace lock");
            if workspaces.contains_key(&record.workspace_id) {
                return Err(VfsCatalogError::AlreadyExists {
                    kind: "workspace",
                    id: record.workspace_id.to_string(),
                });
            }
            let workspace = VfsWorkspaceRecord {
                workspace_id: record.workspace_id,
                display_name: record.display_name,
                base_snapshot_ref: record.base_snapshot_ref,
                head_snapshot_ref: record.head_snapshot_ref,
                head_totals: record.head_totals,
                revision: 0,
                created_at_ms: record.created_at_ms,
                updated_at_ms: record.created_at_ms,
            };
            workspaces.insert(workspace.workspace_id.clone(), workspace.clone());
            Ok(workspace)
        }

        async fn read_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            self.workspaces
                .lock()
                .expect("workspace lock")
                .get(workspace_id)
                .cloned()
                .ok_or_else(|| VfsCatalogError::NotFound {
                    kind: "workspace",
                    id: workspace_id.to_string(),
                })
        }

        async fn list_workspaces(&self) -> Result<Vec<VfsWorkspaceRecord>, VfsCatalogError> {
            Ok(self
                .workspaces
                .lock()
                .expect("workspace lock")
                .values()
                .cloned()
                .collect())
        }

        async fn compare_and_set_head(
            &self,
            request: CompareAndSetVfsWorkspaceHead,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            let mut workspaces = self.workspaces.lock().expect("workspace lock");
            let workspace = workspaces.get_mut(&request.workspace_id).ok_or_else(|| {
                VfsCatalogError::NotFound {
                    kind: "workspace",
                    id: request.workspace_id.to_string(),
                }
            })?;
            if let Some(display_name) = request.display_name {
                workspace.display_name = Some(display_name);
            }
            workspace.head_snapshot_ref = request.new_head_snapshot_ref;
            workspace.head_totals = request.new_head_totals;
            workspace.revision += 1;
            workspace.updated_at_ms = request.updated_at_ms;
            Ok(workspace.clone())
        }

        async fn delete_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            self.workspaces
                .lock()
                .expect("workspace lock")
                .remove(workspace_id)
                .ok_or_else(|| VfsCatalogError::NotFound {
                    kind: "workspace",
                    id: workspace_id.to_string(),
                })
        }
    }
}
