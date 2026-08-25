//! One sub-agent delegation, supervised: started on call by the parent
//! session's `agent_run` / `agent_spawn` invocation (P100b start-on-call),
//! it creates the child session from the pinned profile, waits for the
//! child's run terminal, resolves the parent's `reply` promise with the
//! result envelope, and closes the child. Cancellation from any direction
//! closes the child. To the parent this is indistinguishable from an
//! environment job; the session workflow carries no sub-agent code.

use std::time::Duration;

use futures::{FutureExt, pin_mut, select};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{SyncWorkflowContext, WorkflowContext, WorkflowContextView, WorkflowResult};

use crate::{
    AgentSessionWorkflow, SubagentChildRef, SubagentCloseActivityRequest,
    SubagentExecutionPhase, SubagentExecutionSnapshot, SubagentPrepareActivityRequest,
    SubagentPrepareActivityResult, SubagentResolveActivityRequest, SubagentTerminal,
    WorkflowActivities, WorkflowToolRecoveryResult, WorkflowToolStartArgs, activity_options,
};

#[workflow(name = "SubagentExecutionWorkflow")]
#[derive(Default)]
pub struct SubagentExecutionWorkflow {
    snapshot: SubagentExecutionSnapshot,
    /// Identity fixed at start: which promise this execution resolves and
    /// which holder to tell.
    reply_promise_id: Option<engine::PromiseId>,
    holder_workflow_id: Option<String>,
    invocation_id: Option<engine::WorkflowToolInvocationId>,
    universe_id: Option<uuid::Uuid>,
    pending_terminal: Option<SubagentTerminal>,
    holder_cancelled: bool,
    nudged: bool,
}

#[workflow_methods]
impl SubagentExecutionWorkflow {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        start: WorkflowToolStartArgs,
    ) -> WorkflowResult<()> {
        if ctx.workflow_id() != start.execution_id
            || start.universe_id != start.invocation.session_universe_id
        {
            return Err(
                anyhow::anyhow!("subagent execution identity is invalid: workflow id or universe mismatch")
                    .into(),
            );
        }
        let Some(reply_promise_id) = start
            .invocation
            .completion_promises
            .as_ref()
            .and_then(|promises| promises.get(engine::REPLY_COMPLETION_KEY))
            .cloned()
        else {
            return Err(anyhow::anyhow!(
                "subagent execution invocation is missing its reply completion promise"
            )
            .into());
        };
        ctx.state_mut(|state| {
            state.reply_promise_id = Some(reply_promise_id.clone());
            state.holder_workflow_id = Some(start.holder_workflow_id.clone());
            state.invocation_id = Some(start.invocation.invocation_id.clone());
            state.universe_id = Some(start.universe_id);
            state.snapshot.phase = SubagentExecutionPhase::Preparing;
        });

        // A. prepare: validate the pinned grant, reserve the tree slot,
        // create the child from the pinned profile, start its run.
        let prepared = ctx
            .start_activity(
                WorkflowActivities::subagent_prepare,
                SubagentPrepareActivityRequest {
                    start: start.clone(),
                },
                activity_options(),
            )
            .await
            .map_err(|error| anyhow::anyhow!("subagent prepare failed: {error}"))?;
        let (child, deadline_ms) = match prepared {
            SubagentPrepareActivityResult::Prepared { child, deadline_ms } => (child, deadline_ms),
            SubagentPrepareActivityResult::Rejected { error_ref } => {
                let resolution = engine::PromiseResolution::Failed {
                    error_ref: Some(error_ref),
                };
                ctx.state_mut(|state| {
                    state.snapshot.phase = SubagentExecutionPhase::Resolved;
                    state.snapshot.resolution = Some(resolution.clone());
                });
                emit_resolution(ctx, start.universe_id, &reply_promise_id, resolution).await;
                return Ok(());
            }
        };
        ctx.state_mut(|state| {
            state.snapshot.child = Some(child.clone());
            state.snapshot.phase = SubagentExecutionPhase::Running;
        });

        // B. wait: the child's run terminal, the holder's cancellation, the
        // grant deadline, or Temporal cancellation of this execution.
        let outcome = loop {
            if let Some(terminal) = ctx.state_mut(|state| state.pending_terminal.take()) {
                break WaitOutcome::Terminal(terminal);
            }
            if ctx.state(|state| state.holder_cancelled) {
                break WaitOutcome::HolderCancelled;
            }
            ctx.state_mut(|state| state.nudged = false);
            let wait = ctx.wait_condition(|state| state.nudged);
            let deadline = ctx.timer(Duration::from_millis(deadline_ms.max(1))).fuse();
            let cancelled = ctx.cancelled().fuse();
            pin_mut!(wait, deadline, cancelled);
            select! {
                _ = wait => continue,
                _ = deadline => break WaitOutcome::Terminal(SubagentTerminal::Deadline),
                _ = cancelled => break WaitOutcome::Cancelled,
            }
        };

        match outcome {
            WaitOutcome::Terminal(terminal) => {
                // C. resolve: build the envelope, close the child, tell the
                // holder.
                let resolution = ctx
                    .start_activity(
                        WorkflowActivities::subagent_resolve,
                        SubagentResolveActivityRequest {
                            universe_id: start.universe_id,
                            child: child.clone(),
                            terminal,
                        },
                        activity_options(),
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("subagent resolve failed: {error}"))?;
                ctx.state_mut(|state| {
                    state.snapshot.phase = SubagentExecutionPhase::Resolved;
                    state.snapshot.resolution = Some(resolution.clone());
                });
                emit_resolution(ctx, start.universe_id, &reply_promise_id, resolution).await;
                Ok(())
            }
            WaitOutcome::HolderCancelled => {
                // The parent already marked the promise cancelled; only the
                // child needs closing.
                close_child(ctx, start.universe_id, &child).await;
                ctx.state_mut(|state| state.snapshot.phase = SubagentExecutionPhase::Cancelled);
                Ok(())
            }
            WaitOutcome::Cancelled => {
                close_child(ctx, start.universe_id, &child).await;
                ctx.state_mut(|state| state.snapshot.phase = SubagentExecutionPhase::Cancelled);
                Err(temporalio_sdk::WorkflowTermination::Cancelled)
            }
        }
    }

    /// The child's `run_terminal` (its notify intent names this execution
    /// as holder) and the holder's `invocation_cancellation`.
    #[signal(name = "deliver_emission")]
    pub fn deliver_emission(
        &mut self,
        _ctx: &mut SyncWorkflowContext<Self>,
        envelope: engine::EmissionEnvelope,
    ) {
        match envelope.body {
            engine::EmissionBody::RunTerminal {
                token,
                run_id,
                status,
                output_ref,
                failure_message_ref,
            } => {
                // The token is the reply promise id, unique to this
                // invocation, so it identifies the child's run on its own.
                // The child may finish before the prepare activity's result
                // is recorded here, so the terminal must be accepted even
                // when the child ref is not known yet; when it is known, the
                // producer must be that child.
                let expected_token = self
                    .reply_promise_id
                    .as_ref()
                    .map(|promise| promise.as_str().to_owned());
                let from_child = match (&envelope.producer, self.snapshot.child.as_ref()) {
                    (engine::EmissionProducer::Session { session_id, .. }, Some(child)) => {
                        session_id.as_str() == child.session_id && run_id.as_u64() == child.run_id
                    }
                    (engine::EmissionProducer::Session { .. }, None) => true,
                    (engine::EmissionProducer::Workflow { .. }, _) => false,
                };
                if expected_token.as_deref() == Some(token.as_str())
                    && from_child
                    && self.pending_terminal.is_none()
                {
                    self.pending_terminal = Some(SubagentTerminal::Run {
                        status,
                        output_ref,
                        failure_message_ref,
                    });
                    self.nudged = true;
                }
            }
            engine::EmissionBody::InvocationCancellation {
                invocation_id,
                completion_key,
                ..
            } => {
                let from_holder = match &envelope.producer {
                    engine::EmissionProducer::Session {
                        universe_id,
                        session_id,
                        ..
                    } => {
                        self.holder_workflow_id.as_deref()
                            == Some(crate::compose_workflow_id(*universe_id, session_id).as_str())
                    }
                    engine::EmissionProducer::Workflow { .. } => false,
                };
                if from_holder
                    && self.invocation_id.as_ref() == Some(&invocation_id)
                    && completion_key == engine::REPLY_COMPLETION_KEY
                {
                    self.holder_cancelled = true;
                    self.nudged = true;
                }
            }
            engine::EmissionBody::SourceResolution { .. }
            | engine::EmissionBody::ToolInvocation { .. } => {}
        }
    }

    #[query(name = "snapshot")]
    pub fn snapshot(&self, _ctx: &WorkflowContextView) -> SubagentExecutionSnapshot {
        self.snapshot.clone()
    }

    #[query(name = "workflow_tool_recovery")]
    pub fn workflow_tool_recovery(&self, _ctx: &WorkflowContextView) -> WorkflowToolRecoveryResult {
        let mut resolutions = std::collections::BTreeMap::new();
        if let Some(resolution) = &self.snapshot.resolution {
            resolutions.insert(
                engine::REPLY_COMPLETION_KEY.to_owned(),
                resolution.clone(),
            );
        }
        WorkflowToolRecoveryResult { resolutions }
    }
}

enum WaitOutcome {
    Terminal(SubagentTerminal),
    HolderCancelled,
    Cancelled,
}

async fn emit_resolution(
    ctx: &mut WorkflowContext<SubagentExecutionWorkflow>,
    universe_id: uuid::Uuid,
    reply_promise_id: &engine::PromiseId,
    resolution: engine::PromiseResolution,
) {
    let Some(holder) = ctx.state(|state| state.holder_workflow_id.clone()) else {
        return;
    };
    let envelope = engine::EmissionEnvelope::source_resolution(
        universe_id,
        ctx.workflow_id().to_owned(),
        reply_promise_id.clone(),
        resolution,
    );
    let _ = ctx
        .external_workflow(holder, None)
        .signal(AgentSessionWorkflow::deliver_emission, envelope)
        .await;
}

async fn close_child(
    ctx: &mut WorkflowContext<SubagentExecutionWorkflow>,
    universe_id: uuid::Uuid,
    child: &SubagentChildRef,
) {
    let _ = ctx
        .start_activity(
            WorkflowActivities::subagent_close,
            SubagentCloseActivityRequest {
                universe_id,
                session_id: child.session_id.clone(),
            },
            activity_options(),
        )
        .await;
}
