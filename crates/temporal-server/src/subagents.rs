//! Hosted sub-agent delegation service (P134): the activities behind
//! `SubagentExecutionWorkflow`. Prepare validates the pinned grant, reserves
//! the root-scoped tree slot by creating the child session row, creates the
//! child from the pinned profile, and starts its run with a notify intent
//! back to the execution; resolve builds the result envelope and closes the
//! child; close force-closes a cancelled child.

use std::sync::Arc;

use api::{
    AgentApiError, AgentApiService, AgentProfile, InlineAgentProfile, InputItem, ProfileId,
    ProfileReadParams, ProfileSource, SessionCloseParams,
};
use async_trait::async_trait;
use engine::{
    BlobRef, PromiseResolution, RunStatus, RunTerminalNotifyIntent, SessionId, SubmissionId,
    storage::{
        BlobStore, CreateSession, SessionOrigin, SessionOriginKind, SessionStore,
        SessionStoreError,
    },
};
use temporal_workflow::{
    SubagentChildRef, SubagentPrepareActivityResult, SubagentTerminal, WorkflowToolStartArgs,
};
use tools::{
    concurrency::{AwaitArgs, AwaitModeArg},
    subagents::{
        AgentCallArgs, SubagentExecutionContextV1, SubagentResultEnvelope, SubagentResultStatus,
        SubagentToolKind,
    },
};

use crate::gateway::GatewayAgentApi;

const MAX_SUBAGENT_OUTPUT_BYTES: usize = 512 * 1024;

/// The child-side operations the service needs from the hosted API. A
/// trait so the service can be exercised against a counting fake.
#[async_trait]
pub trait SubagentChildRuntime: Send + Sync {
    async fn read_profile(&self, profile_id: ProfileId) -> Result<AgentProfile, AgentApiError>;

    async fn start_session(
        &self,
        session_id: &SessionId,
        profile: ProfileSource,
    ) -> Result<(), AgentApiError>;

    async fn start_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError>;

    async fn close_session(&self, session_id: &SessionId, force: bool)
    -> Result<(), AgentApiError>;
}

#[derive(Clone)]
pub struct AgentApiSubagentRuntime {
    api: Arc<GatewayAgentApi>,
}

impl AgentApiSubagentRuntime {
    pub fn new(api: Arc<GatewayAgentApi>) -> Self {
        Self { api }
    }
}

#[async_trait]
impl SubagentChildRuntime for AgentApiSubagentRuntime {
    async fn read_profile(&self, profile_id: ProfileId) -> Result<AgentProfile, AgentApiError> {
        Ok(self
            .api
            .read_profile(ProfileReadParams { profile_id })
            .await?
            .result
            .profile)
    }

    async fn start_session(
        &self,
        session_id: &SessionId,
        profile: ProfileSource,
    ) -> Result<(), AgentApiError> {
        self.api
            .start_session_for_subagent(session_id, profile)
            .await
    }

    async fn start_run(
        &self,
        session_id: &SessionId,
        input: Vec<InputItem>,
        submission_id: SubmissionId,
        notify_on_terminal: Vec<RunTerminalNotifyIntent>,
    ) -> Result<String, AgentApiError> {
        self.api
            .start_run_for_subagent(session_id, input, submission_id, notify_on_terminal)
            .await
    }

    async fn close_session(
        &self,
        session_id: &SessionId,
        force: bool,
    ) -> Result<(), AgentApiError> {
        self.api
            .close_session(SessionCloseParams {
                session_id: session_id.as_str().to_owned(),
                force,
            })
            .await
            .map(|_| ())
    }
}

#[derive(Clone)]
pub struct SubagentService {
    sessions: Arc<dyn SessionStore>,
    blobs: Arc<dyn BlobStore>,
    runtime: Arc<dyn SubagentChildRuntime>,
}

impl SubagentService {
    pub fn new(
        sessions: Arc<dyn SessionStore>,
        blobs: Arc<dyn BlobStore>,
        runtime: Arc<dyn SubagentChildRuntime>,
    ) -> Self {
        Self {
            sessions,
            blobs,
            runtime,
        }
    }

    /// Step A of the execution. Every expected failure (limit, unlisted
    /// agent, missing profile) is `Rejected` so the parent's `reply`
    /// promise fails cleanly; only infrastructure failures are errors.
    pub async fn prepare(
        &self,
        start: WorkflowToolStartArgs,
        now_ms: u64,
    ) -> Result<SubagentPrepareActivityResult, AgentApiError> {
        let invocation = &start.invocation;
        let expected_holder =
            temporal_workflow::compose_workflow_id(start.universe_id, &invocation.session_id);
        if start.execution_id.is_empty()
            || start.universe_id != invocation.session_universe_id
            || start.holder_workflow_id != expected_holder
        {
            return Err(AgentApiError::invalid_request(
                "subagent workflow-tool start identity is invalid",
            ));
        }
        let Some(reply_promise_id) = invocation
            .completion_promises
            .as_ref()
            .and_then(|promises| promises.get(engine::REPLY_COMPLETION_KEY))
        else {
            return Err(AgentApiError::invalid_request(
                "subagent invocation is missing its reply completion promise",
            ));
        };
        if SubagentToolKind::from_binding(
            invocation.tool_id.as_str(),
            invocation.semantic_type.as_str(),
        )
        .is_none()
        {
            return Err(AgentApiError::invalid_request(format!(
                "unsupported subagent workflow tool {} ({})",
                invocation.tool_id, invocation.semantic_type
            )));
        }
        let args: AgentCallArgs = self.read_json(&invocation.arguments_ref).await?;
        if let Err(error) = args.validate() {
            return self.rejected(error.to_string()).await;
        }
        let context_ref = invocation.execution_context_ref.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request("subagent invocation is missing its execution context")
        })?;
        let context: SubagentExecutionContextV1 = self.read_json(context_ref).await?;
        if context.version != SubagentExecutionContextV1::VERSION {
            return Err(AgentApiError::invalid_request(format!(
                "unsupported subagent execution context version {}",
                context.version
            )));
        }
        if context.agent_profile_id != args.agent {
            return self
                .rejected(format!(
                    "agent {} does not match the admitted agent {}",
                    args.agent, context.agent_profile_id
                ))
                .await;
        }
        let parent_session_id = SessionId::try_new(context.parent_session_id.clone())
            .map_err(|error| AgentApiError::internal(format!("invalid parent id: {error}")))?;
        let parent = self
            .sessions
            .load_session(&parent_session_id)
            .await
            .map_err(api_projection::map_session_store_error)?
            .ok_or_else(|| {
                AgentApiError::not_found(format!("parent session not found: {parent_session_id}"))
            })?;
        let (root_session_id, depth, limits) = match parent.origin.as_ref() {
            Some(origin) => (
                origin.root_session_id.clone(),
                origin.depth.saturating_add(1),
                context.grant_limits.attenuated_by(origin.limits),
            ),
            None => (parent_session_id.clone(), 1, context.grant_limits),
        };
        let profile_id = match ProfileId::try_new(args.agent.clone()) {
            Ok(profile_id) => profile_id,
            Err(error) => {
                return self
                    .rejected(format!("invalid agent profile id {:?}: {error}", args.agent))
                    .await;
            }
        };
        let profile = match self.runtime.read_profile(profile_id.clone()).await {
            Ok(profile) => profile,
            Err(error) if is_not_found(&error) => {
                return self
                    .rejected(format!("agent profile does not exist: {profile_id}"))
                    .await;
            }
            Err(error) => return Err(error),
        };
        let child_session_id = child_session_id(&start.execution_id);
        let display_name = args.label.clone().or_else(|| {
            profile
                .display_name
                .clone()
                .or_else(|| Some(profile_id.as_str().to_owned()))
        });
        let origin = SessionOrigin {
            kind: SessionOriginKind::Subagent,
            parent_session_id: parent_session_id.clone(),
            parent_run_id: context.parent_run_id,
            root_session_id,
            depth,
            invocation_id: invocation.invocation_id.as_str().to_owned(),
            profile_id: profile_id.as_str().to_owned(),
            profile_revision: profile.revision,
            limits,
        };
        match self
            .sessions
            .create_session(CreateSession {
                session_id: child_session_id.clone(),
                display_name,
                origin: Some(origin.clone()),
                created_at_ms: now_ms,
            })
            .await
        {
            Ok(_) => {}
            Err(SessionStoreError::SessionAlreadyExists { .. }) => {
                // Retry of this execution: the row is ours iff it names this
                // invocation; anything else is an identity collision.
                let existing = self
                    .sessions
                    .load_session(&child_session_id)
                    .await
                    .map_err(api_projection::map_session_store_error)?;
                let ours = existing.as_ref().is_some_and(|record| {
                    record
                        .origin
                        .as_ref()
                        .is_some_and(|existing| existing.invocation_id == origin.invocation_id)
                });
                if !ours {
                    return Err(AgentApiError::conflict(format!(
                        "subagent child session id collides with an unrelated session: {child_session_id}"
                    )));
                }
            }
            Err(error @ SessionStoreError::OriginLimitExceeded { .. }) => {
                return self.rejected(error.to_string()).await;
            }
            Err(error) => return Err(api_projection::map_session_store_error(error)),
        }
        // The profile is applied inline so the child runs the revision that
        // was pinned on its origin, not whatever the registry holds later.
        // A profile that cannot be applied (an `inherit` without a parent
        // environment, a missing binding, ...) is a rejected delegation the
        // parent must see, not an activity retry: a retried start finds the
        // child workflow already running and would skip the profile.
        if let Err(error) = self
            .runtime
            .start_session(
                &child_session_id,
                ProfileSource::Inline {
                    profile: Box::new(InlineAgentProfile {
                        display_name: profile.display_name.clone(),
                        description: profile.description.clone(),
                        document: profile.document.clone(),
                    }),
                },
            )
            .await
        {
            if is_caller_error(&error) {
                let _ = self.close(child_session_id.as_str()).await;
                return self
                    .rejected(format!("agent {profile_id} could not be started: {error}"))
                    .await;
            }
            return Err(error);
        }
        let submission_id =
            SubmissionId::new(format!("subagent_run_{}", digest_suffix(&start.execution_id)));
        let run_id = self
            .runtime
            .start_run(
                &child_session_id,
                vec![InputItem::Text { text: args.input }],
                submission_id,
                vec![RunTerminalNotifyIntent {
                    holder_workflow_id: start.execution_id.clone(),
                    token: reply_promise_id.as_str().to_owned(),
                }],
            )
            .await?;
        let run_id = parse_api_run_id(&run_id)?;
        Ok(SubagentPrepareActivityResult::Prepared {
            child: SubagentChildRef {
                session_id: child_session_id.as_str().to_owned(),
                run_id,
                agent_profile_id: profile_id.as_str().to_owned(),
            },
            deadline_ms: limits.deadline_ms,
        })
    }

    /// Step C: the envelope the parent sees, then the child is closed.
    pub async fn resolve(
        &self,
        child: SubagentChildRef,
        terminal: SubagentTerminal,
    ) -> Result<PromiseResolution, AgentApiError> {
        let (status, output, error) = match terminal {
            SubagentTerminal::Run {
                status,
                output_ref,
                failure_message_ref,
            } => {
                let output = match output_ref.as_ref() {
                    Some(output_ref) => Some(self.read_text(output_ref).await?),
                    None => None,
                };
                let failure = match failure_message_ref.as_ref() {
                    Some(failure_ref) => Some(self.read_text(failure_ref).await?),
                    None => None,
                };
                match status {
                    RunStatus::Completed => (SubagentResultStatus::Completed, output, None),
                    RunStatus::Cancelled => (
                        SubagentResultStatus::Cancelled,
                        output,
                        Some("sub-agent run was cancelled".to_owned()),
                    ),
                    RunStatus::Failed
                    | RunStatus::Active
                    | RunStatus::Parked
                    | RunStatus::Cancelling => (
                        SubagentResultStatus::Failed,
                        output,
                        Some(failure.unwrap_or_else(|| "sub-agent run failed".to_owned())),
                    ),
                }
            }
            SubagentTerminal::Deadline => (
                SubagentResultStatus::Deadline,
                None,
                Some("sub-agent run exceeded the grant deadline".to_owned()),
            ),
        };
        let envelope = SubagentResultEnvelope {
            agent: child.agent_profile_id.clone(),
            session_id: child.session_id.clone(),
            run_id: Some(format!("run_{}", child.run_id)),
            status,
            output,
            error,
        };
        let payload_ref = self
            .blobs
            .put_bytes(serde_json::to_vec(&envelope).map_err(|error| {
                AgentApiError::internal(format!("encode subagent result: {error}"))
            })?)
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        self.close(&child.session_id).await?;
        Ok(match status {
            SubagentResultStatus::Completed => PromiseResolution::Resolved {
                payload_ref: Some(payload_ref),
            },
            SubagentResultStatus::Failed
            | SubagentResultStatus::Cancelled
            | SubagentResultStatus::Deadline => PromiseResolution::Failed {
                error_ref: Some(payload_ref),
            },
        })
    }

    /// Force-close the child; an already-closed or missing child is fine.
    pub async fn close(&self, session_id: &str) -> Result<(), AgentApiError> {
        let session_id = SessionId::try_new(session_id.to_owned())
            .map_err(|error| AgentApiError::internal(format!("invalid child id: {error}")))?;
        match self.runtime.close_session(&session_id, true).await {
            Ok(()) => Ok(()),
            Err(error) if is_not_found(&error) || is_already_closed(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn rejected(
        &self,
        message: String,
    ) -> Result<SubagentPrepareActivityResult, AgentApiError> {
        let error_ref = self
            .blobs
            .put_bytes(
                serde_json::to_vec(&serde_json::json!({ "error": message }))
                    .map_err(|error| AgentApiError::internal(error.to_string()))?,
            )
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        Ok(SubagentPrepareActivityResult::Rejected { error_ref })
    }

    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        blob_ref: &BlobRef,
    ) -> Result<T, AgentApiError> {
        let bytes = self
            .blobs
            .read_bytes(blob_ref)
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| AgentApiError::invalid_request(format!("invalid JSON blob: {error}")))
    }

    /// Run output blobs hold either a JSON string, another JSON value, or
    /// raw text; the envelope carries text either way, bounded.
    async fn read_text(&self, blob_ref: &BlobRef) -> Result<String, AgentApiError> {
        let bytes = self
            .blobs
            .read_bytes(blob_ref)
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        let text = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(serde_json::Value::String(text)) => text,
            Ok(value) => value.to_string(),
            Err(_) => String::from_utf8_lossy(&bytes).into_owned(),
        };
        Ok(truncate_utf8(text, MAX_SUBAGENT_OUTPUT_BYTES))
    }
}

/// Generic `await` argument evaluation shared by the session tool runtime.
pub fn await_spec_from_args(
    args: AwaitArgs,
    now_ms: u64,
) -> Result<engine::AwaitSpec, AgentApiError> {
    let promise_ids = args
        .validated_promise_ids()
        .map_err(|error| AgentApiError::invalid_request(error.to_string()))?
        .into_iter()
        .map(engine::PromiseId::new)
        .collect();
    Ok(engine::AwaitSpec {
        promise_ids,
        mode: match args.mode {
            AwaitModeArg::All => engine::AwaitMode::All,
            AwaitModeArg::Any => engine::AwaitMode::Any,
        },
        deadline_at_ms: args
            .timeout_ms
            .map(|timeout| now_ms.saturating_add(timeout)),
        mailbox: args.mailbox,
    })
}

pub fn child_session_id(execution_id: &str) -> SessionId {
    SessionId::new(format!("agent_{}", digest_suffix(execution_id)))
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

fn parse_api_run_id(run_id: &str) -> Result<u64, AgentApiError> {
    run_id
        .strip_prefix("run_")
        .and_then(|rest| rest.parse::<u64>().ok())
        .ok_or_else(|| {
            AgentApiError::internal(format!(
                "subagent runtime returned malformed run id: {run_id}"
            ))
        })
}

fn truncate_utf8(mut text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let mut cut = max_bytes;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str("\n…[truncated]");
    text
}

fn is_not_found(error: &AgentApiError) -> bool {
    matches!(error.kind, api::AgentApiErrorKind::NotFound)
}

/// Errors that describe the request rather than the runtime: surfaced to
/// the parent as a rejected delegation instead of retried.
fn is_caller_error(error: &AgentApiError) -> bool {
    matches!(
        error.kind,
        api::AgentApiErrorKind::NotFound
            | api::AgentApiErrorKind::InvalidRequest
            | api::AgentApiErrorKind::Rejected
            | api::AgentApiErrorKind::Conflict
    )
}

fn is_already_closed(error: &AgentApiError) -> bool {
    matches!(error.kind, api::AgentApiErrorKind::Rejected) && error.to_string().contains("closed")
}
