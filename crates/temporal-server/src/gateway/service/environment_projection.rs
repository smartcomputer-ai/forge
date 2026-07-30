use super::*;

impl GatewayAgentApi {
    pub(super) async fn refresh_environment_projection_for_idle_session(
        &self,
        session_id: &SessionId,
        state: &engine::CoreAgentState,
    ) -> Result<(), AgentApiError> {
        if state.lifecycle.status != CoreAgentStatus::Open
            || state.runs.active.is_some()
            || !state.runs.queued.is_empty()
        {
            return Ok(());
        }
        let commands = self.environment_projection_refresh_commands(state).await?;
        if commands.is_empty() {
            return Ok(());
        }
        let expected = commands
            .iter()
            .filter_map(|command| match command {
                CoreAgentCommand::UpsertContext { key, entry, .. } => {
                    Some((key.clone(), entry.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let removed = commands
            .iter()
            .filter_map(|command| match command {
                CoreAgentCommand::RemoveContext { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut correlations = BTreeMap::new();
        for command in commands {
            correlations.extend(
                self.submit_correlated_context_commands(session_id, vec![command])
                    .await?,
            );
        }
        if !expected.is_empty() {
            self.wait_for_context_entries_applied(session_id, &expected, &correlations)
                .await?;
        }
        if !removed.is_empty() {
            let (_, outcomes) = self
                .wait_for_context_keys_removed(session_id, &removed, &correlations)
                .await?;
            if let Some(failure) = outcomes.into_values().flatten().next() {
                return Err(map_admission_failure_to_api_error(&failure));
            }
        }
        Ok(())
    }

    pub(super) async fn environment_projection_refresh_commands(
        &self,
        state: &engine::CoreAgentState,
    ) -> Result<Vec<CoreAgentCommand>, AgentApiError> {
        let enabled = state
            .lifecycle
            .config
            .as_ref()
            .is_some_and(|config| config.features.vfs.is_some());
        if !enabled {
            return Ok(state
                .context
                .entries
                .iter()
                .any(|entry| entry.kind == ContextEntryKind::VfsCatalog)
                .then(|| CoreAgentCommand::RemoveContext {
                    expected_revision: None,
                    key: ContextEntryKey::new(engine::VFS_CATALOG_CONTEXT_KEY),
                })
                .into_iter()
                .collect());
        }
        let links = self.resolve_session_workspace_links(state).await?;
        let catalog = tools::environment::projection::vfs_catalog_from_workspace_links(&links)
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        let publication = tools::environment::projection::prepare_vfs_catalog_publication(
            self.store.as_ref(),
            state,
            catalog,
        )
        .await
        .map_err(|error| AgentApiError::internal(error.to_string()))?;
        Ok(publication.command.into_iter().collect())
    }
}
