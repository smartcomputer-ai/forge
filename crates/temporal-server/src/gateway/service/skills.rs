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
        let target_catalog_ref = match &command {
            CoreAgentCommand::UpsertContext { key, entry, .. }
                if key.as_str() == SKILL_CATALOG_CONTEXT_KEY
                    && matches!(entry.kind, ContextEntryKind::SkillCatalog) =>
            {
                Some(entry.content.content_ref.clone())
            }
            CoreAgentCommand::RemoveContext { key, .. }
                if key.as_str() == SKILL_CATALOG_CONTEXT_KEY =>
            {
                None
            }
            _ => {
                return Err(AgentApiError::internal(
                    "skill catalog refresh produced non-catalog context command",
                ));
            }
        };
        let baseline_failures = self
            .query_status_optional(session_id)
            .await?
            .map(|status| status.admission_failures.len())
            .unwrap_or(0);
        self.submit_core_command(session_id, command).await?;
        self.wait_for_skill_catalog(session_id, target_catalog_ref, baseline_failures)
            .await
    }

    pub(super) async fn skill_catalog_refresh_command(
        &self,
        _session_id: &SessionId,
        state: &engine::CoreAgentState,
    ) -> Result<Option<CoreAgentCommand>, AgentApiError> {
        let active_catalog_ref = active_skill_catalog_ref(state);
        let skills_config = state
            .lifecycle
            .config
            .as_ref()
            .and_then(|config| config.features.vfs.as_ref())
            .and_then(|vfs| vfs.skills.as_ref());
        let Some(skills_config) = skills_config else {
            return Ok(clear_skill_catalog_command(active_catalog_ref.as_ref()));
        };
        let links = self.resolve_session_workspace_links(state).await?;
        let specs = configured_vfs_skill_root_specs(&links, skills_config.roots.as_deref())
            .map_err(|error| AgentApiError::invalid_request(error.to_string()))?;
        if specs.is_empty() {
            return Ok(clear_skill_catalog_command(active_catalog_ref.as_ref()));
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
            return Ok(clear_skill_catalog_command(active_catalog_ref.as_ref()));
        }

        let mut state = engine::CoreAgentState::new();
        if let Some(catalog_ref) = active_catalog_ref {
            state.context.entries = vec![active_catalog_entry(catalog_ref)];
        }
        let publication = tools::skills::prepare_skill_catalog_publication_with_warnings(
            self.store.as_ref(),
            Some(self.store.as_ref()),
            &state,
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
        let Some(catalog_ref) = active_skill_catalog_ref(&loaded.state) else {
            return Ok(SkillListResponse {
                catalog_ref: None,
                skills: Vec::new(),
            });
        };
        let catalog = self.read_skill_catalog(&catalog_ref).await?;
        Ok(skill_list_response(Some(&catalog_ref), Some(&catalog)))
    }

    pub(super) async fn read_skill_catalog(
        &self,
        catalog_ref: &BlobRef,
    ) -> Result<SkillCatalogSnapshot, AgentApiError> {
        let bytes = self
            .store
            .read_bytes(catalog_ref)
            .await
            .map_err(map_blob_read_error)?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AgentApiError::internal(format!("stored skill catalog is invalid JSON: {error}"))
        })
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

    pub(super) async fn wait_for_skill_catalog(
        &self,
        session_id: &SessionId,
        target_catalog_ref: Option<BlobRef>,
        baseline_failures: usize,
    ) -> Result<(), AgentApiError> {
        let started = Instant::now();
        loop {
            if started.elapsed() > self.operation_timeout {
                return Err(AgentApiError::internal(format!(
                    "timed out waiting for skill catalog update: {session_id}"
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
            let actual = active_skill_catalog_ref(&loaded.state);
            if actual == target_catalog_ref {
                return Ok(());
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

pub(super) fn clear_skill_catalog_command(
    active_catalog_ref: Option<&BlobRef>,
) -> Option<CoreAgentCommand> {
    active_catalog_ref.map(|_| CoreAgentCommand::RemoveContext {
        expected_revision: None,
        key: ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY),
    })
}

pub(super) fn active_catalog_entry(catalog_ref: BlobRef) -> ContextEntry {
    let input = skill_catalog_context_input(catalog_ref);
    ContextEntry {
        entry_id: engine::ContextEntryId::new(1),
        key: Some(ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY)),
        kind: ContextEntryKind::SkillCatalog,
        source: engine::ContextEntrySource::Runtime {
            label: "skills.catalog.vfs".to_owned(),
        },
        content: input.content,
        preview: input.preview,
        origin: input.origin,
        provenance_ref: input.provenance_ref,
        token_estimate: input.token_estimate,
        supersedes: None,
    }
}

pub(super) fn active_skill_catalog_ref(state: &engine::CoreAgentState) -> Option<BlobRef> {
    state
        .context
        .entries
        .iter()
        .rev()
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|key| key.as_str() == SKILL_CATALOG_CONTEXT_KEY)
                && matches!(entry.kind, ContextEntryKind::SkillCatalog)
        })
        .map(|entry| entry.content.content_ref.clone())
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
