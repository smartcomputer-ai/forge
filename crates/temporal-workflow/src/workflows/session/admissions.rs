use super::*;

pub(super) async fn process_admissions(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    args: &AgentSessionArgs,
    admissions: Vec<AgentAdmission>,
) -> anyhow::Result<DriveOutcome> {
    let mut drive = drive_from_state(ctx)?;
    admit_admissions(ctx, &mut drive, admissions).await?;
    drive_until_idle(ctx, args, &mut drive).await
}

/// Admit a batch of client admissions against the live drive, appending the
/// resulting events. Used by the outer loop and from inside
/// the drive loop at every action boundary and while an activity is in
/// flight, so cancel/steer/queue land promptly instead of after the run.
pub(super) async fn admit_admissions(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
    admissions: Vec<AgentAdmission>,
) -> anyhow::Result<()> {
    for admission in admissions {
        let correlation_token = admission.correlation_token.clone();
        let mut command = admission.command;
        if command_needs_input_preprocessing(&command) {
            let session_id = drive.session_id().clone();
            match preprocess_input_entries(ctx, session_id, command).await? {
                RunInputPreprocessResult::Succeeded { command: rewritten } => {
                    command = *rewritten;
                }
                RunInputPreprocessResult::Failed { failure } => {
                    record_admission_failure(
                        ctx,
                        failure.with_correlation_token(correlation_token.clone()),
                    );
                    continue;
                }
            }
        }
        if should_refresh_runtime_projection_before_admitting(drive.state(), &command) {
            refresh_runtime_projection_before_run(ctx, &mut *drive).await?;
        }
        match admit_and_append_command(ctx, drive, command, correlation_token).await? {
            CommandAdmissionResult::Accepted => {}
            CommandAdmissionResult::Rejected(failure) => {
                record_admission_failure(ctx, failure);
            }
        }
    }
    Ok(())
}

/// Take and admit the pending admissions that may land right now against
/// the live drive. Returns whether anything was admitted (accepted or
/// rejected). Two classes are held back, in order, for a later drain:
///
/// - everything while a standalone context compaction is pending (run
///   requests would be rejected against that transient state; compaction
///   only runs while no run is active, so nothing time-critical waits);
/// - context/config/tool mutations while a turn's generation is in flight.
///   That turn's request is frozen at its planned revisions and the runtime
///   re-derives it from state, so those revisions must not move until the
///   turn completes. Run control (cancel, steer, queue, promise and
///   workflow-tool facts) carries no such revision and lands immediately.
pub(super) async fn drain_pending_admissions(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
) -> anyhow::Result<bool> {
    if drive.state().context.pending_compaction {
        return Ok(false);
    }
    let turn_in_flight = turn_in_flight(drive.state());
    let admissions = ctx.state_mut(|state| {
        if !turn_in_flight {
            return std::mem::take(&mut state.pending_admissions);
        }
        let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending_admissions)
            .into_iter()
            .partition(|admission| admissible_during_turn(&admission.command));
        state.pending_admissions = later;
        now
    });
    if admissions.is_empty() {
        return Ok(false);
    }
    admit_admissions(ctx, drive, admissions).await?;
    Ok(true)
}

/// Pending admissions that `drain_pending_admissions` would admit now.
pub(super) fn has_admissible_admissions(state: &AgentSessionWorkflow) -> bool {
    if state.core_state.context.pending_compaction {
        return false;
    }
    if !turn_in_flight(&state.core_state) {
        return !state.pending_admissions.is_empty();
    }
    state
        .pending_admissions
        .iter()
        .any(|admission| admissible_during_turn(&admission.command))
}

fn turn_in_flight(state: &CoreAgentState) -> bool {
    state
        .runs
        .active
        .as_ref()
        .is_some_and(|run| run.active_turn_id.is_some())
}

/// Commands that do not move the config/context/toolset revisions an
/// in-flight turn was planned against.
fn admissible_during_turn(command: &CoreAgentCommand) -> bool {
    matches!(
        command,
        CoreAgentCommand::CancelRun { .. }
            | CoreAgentCommand::ForceCancelRun { .. }
            | CoreAgentCommand::RequestRunSteering { .. }
            | CoreAgentCommand::RequestRun(_)
            | CoreAgentCommand::ResolvePromise { .. }
            | CoreAgentCommand::FailWorkflowToolDelivery { .. }
            | CoreAgentCommand::FailWorkflowToolStart { .. }
            | CoreAgentCommand::ResumeToolBatch(_)
            | CoreAgentCommand::CloseSession { .. }
    )
}

enum RunInputPreprocessResult {
    Succeeded { command: Box<CoreAgentCommand> },
    Failed { failure: AgentAdmissionFailure },
}

pub(super) fn command_needs_input_preprocessing(command: &CoreAgentCommand) -> bool {
    match command {
        CoreAgentCommand::RequestRun(request) => request.source.input().iter().any(is_audio_input),
        CoreAgentCommand::UpsertContext { entry, .. } => is_audio_input(entry),
        _ => false,
    }
}

fn is_audio_input(input: &ContextEntryInput) -> bool {
    input
        .media_type
        .as_deref()
        .map(|mime| mime.trim().to_ascii_lowercase().starts_with("audio/"))
        .unwrap_or(false)
}

async fn preprocess_input_entries(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    session_id: SessionId,
    command: CoreAgentCommand,
) -> anyhow::Result<RunInputPreprocessResult> {
    let (submission_id, input, rebuild) = match command {
        CoreAgentCommand::RequestRun(request) => match request.source {
            engine::RunRequestSource::Input { input } => (
                request.submission_id.clone(),
                input,
                InputPreprocessRebuild::RequestRun {
                    submission_id: request.submission_id,
                    run_config: request.run_config,
                    notify_on_terminal: request.notify_on_terminal,
                },
            ),
            engine::RunRequestSource::Context { .. } => {
                return Ok(RunInputPreprocessResult::Succeeded {
                    command: Box::new(CoreAgentCommand::RequestRun(request)),
                });
            }
        },
        CoreAgentCommand::UpsertContext {
            expected_revision,
            key,
            entry,
        } => (
            None,
            vec![entry],
            InputPreprocessRebuild::UpsertContext {
                expected_revision,
                key,
            },
        ),
        command => {
            return Ok(RunInputPreprocessResult::Succeeded {
                command: Box::new(command),
            });
        }
    };

    let result = ctx
        .start_activity(
            WorkflowActivities::preprocess_run_input,
            PreprocessRunInputActivityRequest { session_id, input },
            activity_options(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    match result.outcome {
        PreprocessRunInputOutcome::Succeeded { input } => Ok(RunInputPreprocessResult::Succeeded {
            command: Box::new(rebuild.rebuild(input)?),
        }),
        PreprocessRunInputOutcome::Failed { failure } => Ok(RunInputPreprocessResult::Failed {
            failure: preprocess_failure_to_admission_failure(submission_id, failure),
        }),
    }
}

enum InputPreprocessRebuild {
    RequestRun {
        submission_id: Option<SubmissionId>,
        run_config: RunConfig,
        notify_on_terminal: Vec<engine::RunTerminalNotifyIntent>,
    },
    UpsertContext {
        expected_revision: Option<u64>,
        key: ContextEntryKey,
    },
}

impl InputPreprocessRebuild {
    fn rebuild(self, input: Vec<ContextEntryInput>) -> anyhow::Result<CoreAgentCommand> {
        match self {
            Self::RequestRun {
                submission_id,
                run_config,
                notify_on_terminal,
            } => Ok(CoreAgentCommand::RequestRun(engine::RunRequestCommand {
                notify_on_terminal,
                submission_id,
                source: engine::RunRequestSource::Input { input },
                run_config,
            })),
            Self::UpsertContext {
                expected_revision,
                key,
            } => {
                let mut input = input;
                let Some(entry) = input.pop() else {
                    anyhow::bail!("preprocessed context append returned no entry");
                };
                if !input.is_empty() {
                    anyhow::bail!("preprocessed context append returned multiple entries");
                }
                Ok(CoreAgentCommand::UpsertContext {
                    expected_revision,
                    key,
                    entry,
                })
            }
        }
    }
}

pub(super) fn preprocess_failure_to_admission_failure(
    submission_id: Option<SubmissionId>,
    failure: PreprocessRunInputFailure,
) -> AgentAdmissionFailure {
    AgentAdmissionFailure {
        submission_id,
        correlation_token: None,
        kind: match failure.kind {
            PreprocessRunInputFailureKind::UnsupportedAudioMime => {
                AgentAdmissionFailureKind::UnsupportedAudioMime
            }
            PreprocessRunInputFailureKind::AudioBlobMissing => {
                AgentAdmissionFailureKind::AudioBlobMissing
            }
            PreprocessRunInputFailureKind::AudioBlobTooLarge => {
                AgentAdmissionFailureKind::AudioBlobTooLarge
            }
            PreprocessRunInputFailureKind::AudioDurationTooLong => {
                AgentAdmissionFailureKind::AudioDurationTooLong
            }
            PreprocessRunInputFailureKind::TranscoderUnavailable => {
                AgentAdmissionFailureKind::TranscoderUnavailable
            }
            PreprocessRunInputFailureKind::TranscodeFailure => {
                AgentAdmissionFailureKind::TranscodeFailure
            }
            PreprocessRunInputFailureKind::TranscriptionFailure => {
                AgentAdmissionFailureKind::TranscriptionFailure
            }
        },
        message: failure.message,
        rejection: None,
    }
}

fn should_refresh_runtime_projection_before_admitting(
    state: &CoreAgentState,
    command: &CoreAgentCommand,
) -> bool {
    matches!(command, CoreAgentCommand::RequestRun(_))
        && state.runs.active.is_none()
        && state.runs.queued.is_empty()
}

async fn refresh_runtime_projection_before_run(
    ctx: &mut WorkflowContext<AgentSessionWorkflow>,
    drive: &mut CoreAgentDrive,
) -> anyhow::Result<()> {
    let vfs = drive
        .state()
        .lifecycle
        .config
        .as_ref()
        .and_then(|config| config.features.vfs.as_ref());
    let vfs_catalog_enabled = vfs.is_some();
    let vfs_skills_enabled = vfs.is_some_and(|vfs| vfs.skills.is_some());
    let vfs_prompts_enabled = vfs.is_some_and(|vfs| vfs.prompts.is_some());
    let vfs_prompt_roots = vfs
        .and_then(|vfs| vfs.prompts.as_ref())
        .and_then(|prompts| prompts.roots.clone());
    let vfs_skill_roots = vfs
        .and_then(|vfs| vfs.skills.as_ref())
        .and_then(|skills| skills.roots.clone());
    let result = ctx
        .start_activity(
            WorkflowActivities::runtime_projection_refresh,
            RuntimeProjectionRefreshActivityRequest {
                session_id: drive.session_id().clone(),
                workspace_links: vfs
                    .map(|vfs| vfs.workspace_links.clone())
                    .unwrap_or_default(),
                vfs_catalog_enabled,
                vfs_prompts_enabled,
                vfs_prompt_roots,
                active_instruction_inputs: active_instruction_inputs(drive.state()),
                vfs_skills_enabled,
                vfs_skill_roots,
                active_catalog_ref: active_skill_catalog_ref(drive.state()),
                active_vfs_catalog_ref: active_context_ref(
                    drive.state(),
                    VFS_CATALOG_CONTEXT_KEY,
                    ContextEntryKind::VfsCatalog,
                ),
                subagents: drive
                    .state()
                    .lifecycle
                    .config
                    .as_ref()
                    .and_then(|config| config.features.subagents.clone()),
                active_subagent_catalog_ref: active_context_ref(
                    drive.state(),
                    engine::SUBAGENT_CATALOG_CONTEXT_KEY,
                    ContextEntryKind::SubagentCatalog,
                ),
            },
            activity_options(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    for command in result.commands {
        match admit_and_append_command(ctx, drive, command, None).await? {
            CommandAdmissionResult::Accepted => {}
            CommandAdmissionResult::Rejected(failure) => {
                anyhow::bail!("run context refresh command rejected: {}", failure.message)
            }
        }
    }
    Ok(())
}

fn active_skill_catalog_ref(state: &CoreAgentState) -> Option<BlobRef> {
    active_context_ref(
        state,
        SKILL_CATALOG_CONTEXT_KEY,
        ContextEntryKind::SkillCatalog,
    )
}

fn active_instruction_inputs(
    state: &CoreAgentState,
) -> BTreeMap<ContextEntryKey, ContextEntryInput> {
    state
        .context
        .entries
        .iter()
        .filter(|entry| matches!(entry.kind, ContextEntryKind::Instructions))
        .filter_map(|entry| {
            let key = entry.key.clone()?;
            (key.as_str() == "instructions" || key.as_str().starts_with("instructions.")).then(
                || {
                    (
                        key,
                        ContextEntryInput {
                            kind: entry.kind.clone(),
                            content_ref: entry.content_ref.clone(),
                            media_type: entry.media_type.clone(),
                            preview: entry.preview.clone(),
                            provider_kind: entry.provider_kind.clone(),
                            provider_item_id: entry.provider_item_id.clone(),
                            token_estimate: entry.token_estimate.clone(),
                        },
                    )
                },
            )
        })
        .collect()
}

fn active_context_ref(
    state: &CoreAgentState,
    key: &'static str,
    kind: ContextEntryKind,
) -> Option<BlobRef> {
    state
        .context
        .entries
        .iter()
        .rev()
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|entry_key| entry_key.as_str() == key)
                && entry.kind == kind
        })
        .map(|entry| entry.content_ref.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_preprocess_rebuild_preserves_expected_context_revision() {
        let key = ContextEntryKey::new("client.audio");
        let entry = ContextEntryInput {
            kind: engine::ContextEntryKind::ProviderOpaque,
            content_ref: BlobRef::from_bytes(b"transcribed"),
            media_type: Some("application/json".to_owned()),
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
        };

        let command = InputPreprocessRebuild::UpsertContext {
            expected_revision: Some(7),
            key: key.clone(),
        }
        .rebuild(vec![entry.clone()])
        .expect("rebuild upsert");

        assert_eq!(
            command,
            CoreAgentCommand::UpsertContext {
                expected_revision: Some(7),
                key,
                entry,
            }
        );
    }
}
