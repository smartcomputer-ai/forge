use super::*;

impl GatewayAgentApi {
    pub(super) async fn load_session_state_with_current_skill_catalog(
        &self,
        session_id: &SessionId,
    ) -> Result<LoadedSession, AgentApiError> {
        let loaded = self.load_session_state(session_id).await?;
        if loaded.state.lifecycle.status == CoreAgentStatus::Open
            && loaded.state.runs.active.is_none()
            && loaded.state.runs.queued.is_empty()
        {
            self.refresh_environment_projection_for_idle_session(session_id, &loaded.state)
                .await?;
            let loaded = self.load_session_state(session_id).await?;
            self.refresh_skill_catalog_for_idle_session(session_id, &loaded.state)
                .await?;
            return self.load_session_state(session_id).await;
        }
        Ok(loaded)
    }

    pub(super) async fn refresh_skill_catalog_for_idle_session(
        &self,
        session_id: &SessionId,
        state: &engine::CoreAgentState,
    ) -> Result<(), AgentApiError> {
        let Some(command) = self
            .skill_catalog_refresh_command(session_id, state)
            .await?
        else {
            return Ok(());
        };
        self.apply_catalog_refresh_commands(session_id, vec![command])
            .await
    }

    pub(super) async fn skill_catalog_refresh_command(
        &self,
        _session_id: &SessionId,
        state: &engine::CoreAgentState,
    ) -> Result<Option<CoreAgentCommand>, AgentApiError> {
        let catalogs = engine::current_catalog_inputs(state);
        let current = catalogs.get(&ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY));
        let skills_config = state
            .lifecycle
            .config
            .as_ref()
            .and_then(|config| config.features.vfs.as_ref())
            .and_then(|vfs| vfs.skills.as_ref());
        let Some(skills_config) = skills_config else {
            return Ok(tools::catalog::clear_catalog_command(
                current,
                SKILL_CATALOG_CONTEXT_KEY,
            ));
        };
        let links = self.resolve_session_workspace_links(state).await?;
        let specs = configured_vfs_skill_root_specs(&links, skills_config.roots.as_deref())
            .map_err(|error| AgentApiError::invalid_request(error.to_string()))?;
        if specs.is_empty() {
            return Ok(tools::catalog::clear_catalog_command(
                current,
                SKILL_CATALOG_CONTEXT_KEY,
            ));
        }

        let blobs: Arc<dyn BlobStore> = self.store.clone();
        let workspace_store: Arc<dyn VfsWorkspaceStore> = self.store.clone();
        let resolved = resolve_linked_vfs_skill_roots(blobs, workspace_store, links, specs)
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        let inputs = resolved
            .existing_directory_inputs()
            .await
            .map_err(|error| AgentApiError::internal(error.to_string()))?;
        if inputs.is_empty() && resolved.warnings().is_empty() {
            return Ok(tools::catalog::clear_catalog_command(
                current,
                SKILL_CATALOG_CONTEXT_KEY,
            ));
        }

        let publication = tools::skills::prepare_skill_catalog_publication_with_warnings(
            self.store.as_ref(),
            Some(self.store.as_ref()),
            current,
            &inputs,
            resolved.warnings().to_vec(),
        )
        .await
        .map_err(|error| AgentApiError::internal(error.to_string()))?;
        Ok(publication.command)
    }

    pub(super) async fn project_skill_list(
        &self,
        loaded: &LoadedSession,
    ) -> Result<SkillListResponse, AgentApiError> {
        skill_list_from_context(self.store.as_ref(), &loaded.state).await
    }

    pub(super) fn require_open_idle_session(
        &self,
        session_id: &SessionId,
        loaded: &LoadedSession,
        operation: &str,
    ) -> Result<(), AgentApiError> {
        if loaded.state.lifecycle.status != CoreAgentStatus::Open {
            return Err(AgentApiError::rejected(format!(
                "session is not open: {session_id}"
            )));
        }
        if loaded.state.runs.active.is_some() || !loaded.state.runs.queued.is_empty() {
            return Err(AgentApiError::rejected(format!(
                "{operation} can only change while no run is active or queued"
            )));
        }
        Ok(())
    }
}

pub(super) fn skill_list_response(
    catalog_ref: Option<&BlobRef>,
    catalog: Option<&SkillCatalogSnapshot>,
) -> SkillListResponse {
    let Some(catalog) = catalog else {
        return SkillListResponse {
            catalog_ref: None,
            skills: Vec::new(),
        };
    };
    SkillListResponse {
        catalog_ref: catalog_ref.map(|catalog_ref| catalog_ref.as_str().to_owned()),
        skills: catalog
            .skills
            .iter()
            .map(|skill| SkillListItem {
                skill_id: skill.skill_id.as_str().to_owned(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                short_description: skill.short_description.clone(),
                enabled: skill.enabled,
                location: match &skill.location {
                    SkillLocation::LinkedSnapshot {
                        skill_dir_path,
                        skill_doc_path,
                        ..
                    }
                    | SkillLocation::LinkedWorkspace {
                        skill_dir_path,
                        skill_doc_path,
                        ..
                    } => SkillLocationView::Vfs {
                        skill_dir_path: skill_dir_path.to_string(),
                        skill_doc_path: skill_doc_path.to_string(),
                    },
                },
            })
            .collect(),
    }
}

pub(super) async fn skill_list_from_context(
    blobs: &dyn BlobStore,
    state: &engine::CoreAgentState,
) -> Result<SkillListResponse, AgentApiError> {
    let Some(entry) =
        engine::current_context_entry(state, &ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY))
    else {
        return Ok(SkillListResponse {
            catalog_ref: None,
            skills: Vec::new(),
        });
    };
    let catalog_ref = entry
        .provenance_ref
        .as_ref()
        .ok_or_else(|| AgentApiError::internal("skill catalog is missing its structured source"))?;
    let catalog = {
        let bytes = blobs
            .read_bytes(catalog_ref)
            .await
            .map_err(map_blob_read_error)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AgentApiError::internal(format!("stored skill catalog is invalid JSON: {error}"))
        })?
    };
    Ok(skill_list_response(Some(catalog_ref), Some(&catalog)))
}
