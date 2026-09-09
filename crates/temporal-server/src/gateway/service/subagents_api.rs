use super::*;

impl GatewayAgentApi {
    /// Sub-agent catalog refresh for an idle session: the same
    /// publish-if-changed shape as the skill catalog, computed from the
    /// admitted grant and the current profile records.
    pub(super) async fn refresh_subagent_catalog_for_idle_session(
        &self,
        session_id: &SessionId,
        state: &engine::CoreAgentState,
    ) -> Result<(), AgentApiError> {
        let catalogs = engine::current_catalog_inputs(state);
        let current = catalogs.get(&ContextEntryKey::new(SUBAGENT_CATALOG_CONTEXT_KEY));
        let subagents = state
            .lifecycle
            .config
            .as_ref()
            .and_then(|config| config.features.subagents.as_ref());
        let command = match subagents {
            Some(subagents) => {
                let profiles: Arc<dyn ::profiles::ProfileStore> = self.store.clone();
                let snapshot =
                    crate::worker::subagent_catalog_snapshot(Some(profiles.as_ref()), subagents)
                        .await;
                tools::subagents::prepare_subagent_catalog_publication(
                    self.store.as_ref(),
                    current,
                    &snapshot,
                )
                .await
                .map_err(|error| AgentApiError::internal(error.to_string()))?
            }
            None => tools::catalog::clear_catalog_command(current, SUBAGENT_CATALOG_CONTEXT_KEY),
        };
        let Some(command) = command else {
            return Ok(());
        };
        self.apply_catalog_refresh_commands(session_id, vec![command])
            .await
    }

    /// `ProfileEnvironment::Inherit`: the delegating parent's active
    /// environment, resolved at apply time from the child's origin.
    pub(super) async fn resolve_inherited_environment(
        &self,
        session_id: &SessionId,
    ) -> Result<api::EnvironmentId, AgentApiError> {
        let record = self
            .store
            .load_session(session_id)
            .await
            .map_err(map_session_store_error)?
            .ok_or_else(|| AgentApiError::not_found(format!("session not found: {session_id}")))?;
        let Some(origin) = record.origin.as_ref() else {
            return Err(AgentApiError::invalid_request(
                "profile environment `inherit` requires a sub-agent session with a delegation origin",
            ));
        };
        let parent = self.load_session_state(&origin.parent_session_id).await?;
        parent
            .state
            .environment
            .active_environment_id
            .as_ref()
            .map(|environment_id| environment_id.as_str().to_owned())
            .ok_or_else(|| {
                AgentApiError::rejected(format!(
                    "profile environment `inherit`: parent session {} has no active environment",
                    origin.parent_session_id
                ))
            })
    }
}

impl GatewayAgentApi {
    /// Activate an inherited environment in a sub-agent: the parent already
    /// passed the activation gate for this environment, so the child copies
    /// the selection after checking only its own grant (the environments
    /// feature and its provider allowlist) and that the environment is not
    /// gone. No reachability probe: a not-ready environment makes the
    /// child's tools wait, exactly as it does for the parent.
    pub(super) async fn apply_inherited_environment(
        &self,
        session_id: &SessionId,
        environment_id: api::EnvironmentId,
    ) -> Result<bool, AgentApiError> {
        let environment_id = parse_registry_environment_id(environment_id)?;
        let loaded = self.load_session_state(session_id).await?;
        if loaded.state.environment.active_environment_id.as_ref() == Some(&environment_id) {
            return Ok(false);
        }
        let feature = loaded
            .state
            .lifecycle
            .config
            .as_ref()
            .and_then(|config| config.features.environments.as_ref())
            .ok_or_else(|| {
                AgentApiError::rejected(
                    "profile environment `inherit` requires the environments feature to be granted",
                )
            })?;
        let registry_id = ::environments::EnvironmentId::try_new(
            environment_id.as_str().to_owned(),
        )
        .map_err(|error| AgentApiError::internal(format!("invalid environment id: {error}")))?;
        let environments: Arc<dyn ::environments::EnvironmentStore> = self.store.clone();
        let record = environments
            .read_environment(&registry_id)
            .await
            .map_err(map_environments_error)?;
        if matches!(
            record.status,
            ::environments::EnvironmentStatus::Closing
                | ::environments::EnvironmentStatus::Closed
                | ::environments::EnvironmentStatus::Failed
        ) {
            return Err(AgentApiError::rejected(format!(
                "profile environment `inherit`: parent environment {environment_id} is {:?}",
                record.status
            )));
        }
        let policy = ::environments::EnvironmentAccessPolicy::new(
            feature.providers.clone(),
            feature.registration_keys.clone(),
        );
        if !policy.allows(&record) {
            return Err(AgentApiError::rejected(format!(
                "profile environment `inherit`: parent environment {environment_id}: {}",
                policy.refusal(&record)
            )));
        }
        let baseline_failures = self
            .query_status_optional(session_id)
            .await?
            .map(|status| status.admission_failures.len())
            .unwrap_or(0);
        self.submit_core_command(
            session_id,
            activate_environment_command(environment_id.clone()),
        )
        .await?;
        self.wait_for_active_environment(session_id, Some(&environment_id), baseline_failures)
            .await?;
        Ok(true)
    }
}
