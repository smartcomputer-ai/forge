use super::*;

impl GatewayAgentApi {
    pub(super) async fn configure_session_toolset(
        &self,
        session_id: &SessionId,
        loaded: &LoadedSession,
        wait_for_reconciliation: bool,
    ) -> Result<SessionView, AgentApiError> {
        let session_config = loaded.state.lifecycle.config.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request(format!("session is missing config: {session_id}"))
        })?;
        let jobs_granted = session_config
            .features
            .environments
            .as_ref()
            .is_some_and(|environments| environments.jobs);
        let subagents_granted = session_config.features.subagents.is_some();
        let environments_granted = session_config.features.environments.is_some();
        let mut refreshed = None;
        if jobs_granted && !super::has_all_core_environment_job_bindings(&loaded.state) {
            self.ensure_core_environment_job_workflow_tools(session_id, &loaded.state)
                .await?;
            refreshed = Some(self.load_session_state(session_id).await?);
        }
        if subagents_granted {
            let current = refreshed.as_ref().unwrap_or(loaded);
            if !super::has_all_core_subagent_bindings(&current.state) {
                self.ensure_core_subagent_workflow_tools(session_id, &current.state)
                    .await?;
                refreshed = Some(self.load_session_state(session_id).await?);
            }
        }
        let loaded = refreshed.as_ref().unwrap_or(loaded);
        let session_config = loaded.state.lifecycle.config.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request(format!("session is missing config: {session_id}"))
        })?;
        let expose_environment_jobs = jobs_granted;
        let expose_subagents = subagents_granted;
        let mut config =
            self.session_toolset_config(session_config, environments_granted, jobs_granted);
        let materialized_workflow_tools = loaded
            .state
            .workflow_tools
            .bindings
            .values()
            .filter(|binding| {
                (expose_environment_jobs || !super::is_core_environment_job_binding(binding))
                    && (expose_subagents || !super::is_core_subagent_binding(binding))
            })
            .collect::<Vec<_>>();
        enable_concurrency_for_workflow_tools(
            &mut config,
            materialized_workflow_tools.iter().copied(),
        );
        let mut toolset = register_toolset(&config)
            .map_err(|error| AgentApiError::internal(format!("build session tools: {error}")))?;
        register_workflow_tools(&mut toolset, materialized_workflow_tools.iter().copied())
            .map_err(|error| {
                AgentApiError::invalid_request(format!("materialize workflow tool tools: {error}"))
            })?;

        // Remote MCP tools are derived from the config's declared links,
        // exactly like the standard toolset is derived from the features.
        let desired_mcp = self.desired_mcp_tools(&session_config.features).await?;
        if let Some(colliding) = materialized_workflow_tools
            .iter()
            .copied()
            .map(|binding| &binding.definition.tool.name)
            .find(|tool_name| desired_mcp.contains_key(*tool_name))
        {
            return Err(AgentApiError::invalid_request(format!(
                "workflow tool tool name {colliding} collides with a remote MCP tool"
            )));
        }
        let expected_tools = toolset
            .tools
            .iter()
            .chain(desired_mcp.iter())
            .map(|(name, tool)| (name.clone(), tool.clone()))
            .collect::<BTreeMap<_, _>>();
        let patch = toolset_reconcile_patch(&loaded.state.tooling.tools, toolset, desired_mcp);

        let baseline_failures = self
            .query_status_optional(session_id)
            .await?
            .map(|status| status.admission_failures.len())
            .unwrap_or(0);
        if !patch.is_empty() {
            self.submit_core_command(
                session_id,
                CoreAgentCommand::PatchTools {
                    expected_revision: Some(loaded.state.tooling.revision),
                    patch,
                },
            )
            .await?;
        }
        if !wait_for_reconciliation {
            return self.project_session_by_id(session_id).await;
        }
        self.wait_for_session_toolset(session_id, expected_tools, baseline_failures)
            .await
    }

    pub(super) async fn wait_for_session_toolset(
        &self,
        session_id: &SessionId,
        expected_tools: BTreeMap<ToolName, engine::ToolSpec>,
        baseline_failures: usize,
    ) -> Result<SessionView, AgentApiError> {
        let started = Instant::now();
        loop {
            if started.elapsed() > self.operation_timeout {
                return Err(AgentApiError::internal(format!(
                    "timed out waiting for session tools to configure: {session_id}"
                )));
            }
            if let Some(status) = self.query_status_optional(session_id).await? {
                if status.admission_failures.len() > baseline_failures
                    && let Some(failure) = status.admission_failures.last()
                {
                    return Err(map_admission_failure_to_api_error(failure));
                }
                if let Some(error) = status.last_error {
                    return Err(AgentApiError::internal(format!(
                        "agent workflow reported error: {error}"
                    )));
                }
            }
            let loaded = self.load_session_state(session_id).await?;
            if loaded.state.tooling.tools == expected_tools {
                return self.project_session_by_id(session_id).await;
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

/// Level-triggered reconciliation: converge the installed tools to what the
/// current config implies (standard toolset from features, remote MCP tools
/// from declared links). Re-running against a converged state is a no-op.
pub(super) fn toolset_reconcile_patch(
    active: &BTreeMap<ToolName, engine::ToolSpec>,
    toolset: RegisteredToolset,
    desired_mcp: BTreeMap<ToolName, engine::ToolSpec>,
) -> engine::ToolPatch {
    let mut remove = Vec::new();
    for tool_name in active.keys() {
        if !toolset.tools.contains_key(tool_name) && !desired_mcp.contains_key(tool_name) {
            remove.push(tool_name.clone());
        }
    }

    let mut upsert = Vec::new();
    for (tool_name, tool) in toolset.tools.into_iter().chain(desired_mcp) {
        if active.get(&tool_name) != Some(&tool) {
            upsert.push(tool);
        }
    }

    engine::ToolPatch { upsert, remove }
}
