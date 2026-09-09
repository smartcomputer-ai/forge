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
        self.apply_catalog_refresh_commands(session_id, commands)
            .await
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
                .any(|entry| {
                    entry
                        .key
                        .as_ref()
                        .is_some_and(|key| key.as_str() == VFS_CATALOG_CONTEXT_KEY)
                })
                .then(|| CoreAgentCommand::RemoveContext {
                    expected_revision: None,
                    key: ContextEntryKey::new(VFS_CATALOG_CONTEXT_KEY),
                })
                .into_iter()
                .collect());
        }
        let links = self.resolve_session_workspace_links(state).await?;
        let catalog = tools::environment::projection::vfs_catalog_from_workspace_links(&links)
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        let publication = tools::environment::projection::prepare_vfs_catalog_publication(
            self.store.as_ref(),
            Some(self.store.as_ref()),
            engine::current_catalog_inputs(state)
                .get(&ContextEntryKey::new(VFS_CATALOG_CONTEXT_KEY)),
            catalog,
        )
        .await
        .map_err(|error| AgentApiError::internal(error.to_string()))?;
        Ok(publication.command.into_iter().collect())
    }
}
