use super::*;

impl GatewayAgentApi {
    pub(super) async fn validate_workspace_link_targets(
        &self,
        features: &engine::FeaturesConfig,
    ) -> Result<(), AgentApiError> {
        let Some(vfs) = features.vfs.as_ref() else {
            return Ok(());
        };
        if vfs.workspace_links.is_empty() {
            return Ok(());
        }
        let blobs: Arc<dyn BlobStore> = self.store.clone();
        let workspace_store: Arc<dyn VfsWorkspaceStore> = self.store.clone();
        let resolved = vfs::resolve_workspace_links(blobs, workspace_store, &vfs.workspace_links)
            .await
            .map_err(map_vfs_catalog_error)?;
        if let Some(link) = resolved.iter().find(|link| !link.is_available()) {
            return Err(AgentApiError::invalid_request(format!(
                "workspace link target at {} is unavailable: {}",
                link.path,
                link.unavailable_reason().unwrap_or("unknown reason")
            )));
        }
        Ok(())
    }

    pub(super) async fn resolve_session_workspace_links(
        &self,
        state: &engine::CoreAgentState,
    ) -> Result<Vec<vfs::ResolvedWorkspaceLink>, AgentApiError> {
        let declarations = state
            .lifecycle
            .config
            .as_ref()
            .and_then(|config| config.features.vfs.as_ref())
            .map(|vfs| vfs.workspace_links.as_slice())
            .unwrap_or_default();
        if declarations.is_empty() {
            return Ok(Vec::new());
        }
        let blobs: Arc<dyn BlobStore> = self.store.clone();
        let workspace_store: Arc<dyn VfsWorkspaceStore> = self.store.clone();
        vfs::resolve_workspace_links(blobs, workspace_store, declarations)
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn create_vfs_workspace_record(
        &self,
        params: VfsWorkspaceCreateParams,
    ) -> Result<VfsWorkspaceRecord, AgentApiError> {
        let (snapshot_ref, head_totals) = match params.snapshot_ref {
            Some(snapshot_ref) => {
                let snapshot_ref = parse_blob_ref(&snapshot_ref)?;
                let manifest = vfs::read_snapshot_manifest(self.store.as_ref(), &snapshot_ref)
                    .await
                    .map_err(map_vfs_read_error)?;
                (snapshot_ref, manifest.totals)
            }
            None => {
                let result = vfs::commit_snapshot_manifest(
                    self.store.as_ref(),
                    vfs::VfsSnapshotManifest::empty(),
                )
                .await
                .map_err(map_vfs_commit_error)?;
                (result.snapshot_ref, result.manifest.totals)
            }
        };
        self.record_vfs_snapshot_if_missing(
            snapshot_ref.clone(),
            VfsSnapshotSource::new("api_snapshot").with_subject("vfs/workspaces/create"),
            None,
        )
        .await?;

        let workspace_id = match params.workspace_id {
            Some(workspace_id) => VfsWorkspaceId::try_new(workspace_id).map_err(|error| {
                AgentApiError::invalid_request(format!("invalid vfs workspace id: {error}"))
            })?,
            None => self.allocate_vfs_workspace_id(),
        };
        self.store
            .create_workspace(CreateVfsWorkspaceRecord {
                workspace_id,
                display_name: params.display_name,
                base_snapshot_ref: Some(snapshot_ref.clone()),
                head_snapshot_ref: snapshot_ref,
                head_totals,
                created_at_ms: now_ms()?,
            })
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn read_vfs_workspace_record(
        &self,
        params: VfsWorkspaceReadParams,
    ) -> Result<VfsWorkspaceRecord, AgentApiError> {
        let workspace_id = parse_vfs_workspace_id(params.workspace_id)?;
        self.store
            .read_workspace(&workspace_id)
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn list_vfs_workspace_records(
        &self,
    ) -> Result<Vec<VfsWorkspaceRecord>, AgentApiError> {
        self.store
            .list_workspaces()
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn update_vfs_workspace_record(
        &self,
        params: VfsWorkspaceUpdateParams,
    ) -> Result<VfsWorkspaceRecord, AgentApiError> {
        let workspace_id = parse_vfs_workspace_id(params.workspace_id)?;
        let snapshot_ref = parse_blob_ref(&params.snapshot_ref)?;
        let manifest = vfs::read_snapshot_manifest(self.store.as_ref(), &snapshot_ref)
            .await
            .map_err(map_vfs_read_error)?;
        self.record_vfs_snapshot_if_missing(
            snapshot_ref.clone(),
            VfsSnapshotSource::new("api_workspace_update").with_subject("vfs/workspaces/update"),
            None,
        )
        .await?;
        self.store
            .compare_and_set_head(CompareAndSetVfsWorkspaceHead {
                workspace_id,
                expected_revision: params.expected_revision,
                display_name: params.display_name,
                new_head_snapshot_ref: snapshot_ref,
                new_head_totals: manifest.totals,
                updated_at_ms: now_ms()?,
            })
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn delete_vfs_workspace_record(
        &self,
        params: VfsWorkspaceDeleteParams,
    ) -> Result<VfsWorkspaceRecord, AgentApiError> {
        let workspace_id = parse_vfs_workspace_id(params.workspace_id)?;
        self.store
            .delete_workspace(&workspace_id)
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn record_vfs_snapshot(
        &self,
        snapshot_ref: BlobRef,
        source: VfsSnapshotSource,
        display_name: Option<String>,
    ) -> Result<(), AgentApiError> {
        self.store
            .record_snapshot(VfsSnapshotRecord {
                snapshot_ref,
                source,
                display_name,
                created_at_ms: now_ms()?,
            })
            .await
            .map_err(map_vfs_catalog_error)
    }

    pub(super) async fn record_vfs_snapshot_if_missing(
        &self,
        snapshot_ref: BlobRef,
        source: VfsSnapshotSource,
        display_name: Option<String>,
    ) -> Result<(), AgentApiError> {
        match self.store.read_snapshot(&snapshot_ref).await {
            Ok(_) => Ok(()),
            Err(VfsCatalogError::NotFound { .. }) => {
                self.record_vfs_snapshot(snapshot_ref, source, display_name)
                    .await
            }
            Err(error) => Err(map_vfs_catalog_error(error)),
        }
    }

    pub(super) fn allocate_vfs_workspace_id(&self) -> VfsWorkspaceId {
        VfsWorkspaceId::new(format!("workspace_{}", uuid::Uuid::new_v4().simple()))
    }

    pub(super) async fn configure_session_toolset(
        &self,
        session_id: &SessionId,
        loaded: &LoadedSession,
    ) -> Result<SessionView, AgentApiError> {
        let session_config = loaded.state.lifecycle.config.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request(format!("session is missing config: {session_id}"))
        })?;
        let jobs_granted = session_config
            .features
            .environments
            .as_ref()
            .is_some_and(|environments| environments.jobs);
        let has_process_environment = self.session_has_process_environment(session_id).await?;
        let has_job_read_environment = self.session_has_job_read_environment(session_id).await?;
        let has_job_start_environment = self.session_has_job_start_environment(session_id).await?;
        let mut refreshed = None;
        if jobs_granted
            && !loaded
                .state
                .workflow_tools
                .bindings
                .values()
                .any(super::is_core_environment_job_start_binding)
        {
            self.ensure_core_environment_job_workflow_tool(session_id, &loaded.state)
                .await?;
            refreshed = Some(self.load_session_state(session_id).await?);
        }
        let loaded = refreshed.as_ref().unwrap_or(loaded);
        let session_config = loaded.state.lifecycle.config.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request(format!("session is missing config: {session_id}"))
        })?;
        let expose_job_start = jobs_granted && has_job_start_environment;
        let target = ToolTarget::from(&session_config.model);
        let mut config = self.session_toolset_config(
            session_config,
            has_process_environment,
            jobs_granted && has_job_read_environment,
        );
        let materialized_workflow_tools = loaded
            .state
            .workflow_tools
            .bindings
            .values()
            .filter(|binding| {
                expose_job_start || !super::is_core_environment_job_start_binding(binding)
            })
            .collect::<Vec<_>>();
        enable_concurrency_for_workflow_tools(
            &mut config,
            materialized_workflow_tools.iter().copied(),
        );
        let fs_tools_enabled = config.builtin.fs.enabled();
        let mut toolset = resolve_toolset(ToolsetEnvironment { target: &target }, &config)
            .map_err(|error| AgentApiError::internal(format!("build session tools: {error}")))?;
        materialize_workflow_tools(&mut toolset, materialized_workflow_tools.iter().copied())
            .map_err(|error| {
                AgentApiError::invalid_request(format!("materialize workflow tool tools: {error}"))
            })?;
        let blobs: Arc<dyn BlobStore> = self.store.clone();
        store_tool_documents(blobs.as_ref(), &toolset.documents).await?;

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
        let mut expected_tools = toolset.tools.keys().cloned().collect::<BTreeSet<_>>();
        expected_tools.extend(desired_mcp.keys().cloned());
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
        if fs_tools_enabled {
            self.submit_core_command(
                session_id,
                CoreAgentCommand::SetDefaultToolTarget {
                    target: ToolTargets::session_fs_execution_target(),
                },
            )
            .await?;
        } else {
            self.submit_core_command(
                session_id,
                CoreAgentCommand::ClearDefaultToolTarget {
                    namespace: tools::targets::FS_TARGET_NAMESPACE.to_owned(),
                },
            )
            .await?;
        }
        self.wait_for_session_toolset(
            session_id,
            expected_tools,
            fs_tools_enabled,
            baseline_failures,
        )
        .await
    }

    pub(super) async fn wait_for_session_toolset(
        &self,
        session_id: &SessionId,
        expected_tools: BTreeSet<ToolName>,
        expect_fs_target: bool,
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
                if status.admission_failures.len() > baseline_failures {
                    if let Some(failure) = status.admission_failures.last() {
                        return Err(map_admission_failure_to_api_error(failure));
                    }
                }
                if let Some(error) = status.last_error {
                    return Err(AgentApiError::internal(format!(
                        "agent workflow reported error: {error}"
                    )));
                }
            }
            let loaded = self.load_session_state(session_id).await?;
            let actual_tools = loaded
                .state
                .tooling
                .tools
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            let target = loaded
                .state
                .tooling
                .routing
                .default_targets
                .get(tools::targets::FS_TARGET_NAMESPACE);
            let target_ready = if expect_fs_target {
                target == Some(&ToolTargets::session_fs_execution_target())
            } else {
                target.is_none()
            };
            if actual_tools == expected_tools && target_ready {
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
    toolset: ResolvedToolset,
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

pub(super) async fn commit_vfs_snapshot(
    store: &dyn BlobStore,
    params: VfsSnapshotCommitParams,
) -> Result<VfsSnapshotCommitResponse, AgentApiError> {
    let manifest: vfs::VfsSnapshotManifest =
        serde_json::from_value(params.manifest).map_err(|error| {
            AgentApiError::invalid_request(format!("invalid vfs snapshot manifest: {error}"))
        })?;
    manifest
        .validate()
        .map_err(|error| AgentApiError::invalid_request(error.to_string()))?;
    validate_vfs_manifest_blob_refs(store, &manifest).await?;
    let totals = manifest.totals.clone();
    let result = vfs::commit_snapshot_manifest(store, manifest)
        .await
        .map_err(map_vfs_commit_error)?;
    Ok(VfsSnapshotCommitResponse {
        snapshot_ref: result.snapshot_ref.as_str().to_owned(),
        files: totals.files,
        bytes: totals.bytes,
    })
}

pub(super) async fn read_vfs_snapshot(
    store: &dyn BlobStore,
    params: VfsSnapshotReadParams,
) -> Result<VfsSnapshotReadResponse, AgentApiError> {
    let snapshot_ref = parse_blob_ref(&params.snapshot_ref)?;
    let manifest = vfs::read_snapshot_manifest(store, &snapshot_ref)
        .await
        .map_err(map_vfs_read_error)?;
    let manifest_value = serde_json::to_value(&manifest)
        .map_err(|error| AgentApiError::internal(format!("failed to encode manifest: {error}")))?;
    Ok(VfsSnapshotReadResponse {
        snapshot_ref: snapshot_ref.as_str().to_owned(),
        files: manifest.totals.files,
        bytes: manifest.totals.bytes,
        manifest: manifest_value,
    })
}

pub(super) fn vfs_workspace_view(record: VfsWorkspaceRecord) -> VfsWorkspaceView {
    VfsWorkspaceView {
        workspace_id: record.workspace_id.as_str().to_owned(),
        display_name: record.display_name,
        base_snapshot_ref: record
            .base_snapshot_ref
            .map(|blob_ref| blob_ref.as_str().to_owned()),
        head_snapshot_ref: record.head_snapshot_ref.as_str().to_owned(),
        files: record.head_totals.files,
        bytes: record.head_totals.bytes,
        revision: record.revision,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

pub(super) async fn store_tool_documents(
    blobs: &dyn BlobStore,
    documents: &[ToolDocument],
) -> Result<(), AgentApiError> {
    for document in documents {
        let blob_ref = blobs
            .put_bytes(document.blob_bytes())
            .await
            .map_err(map_blob_store_error)?;
        if blob_ref != document.blob_ref {
            return Err(AgentApiError::internal(format!(
                "tool document blob ref mismatch: expected {}, got {}",
                document.blob_ref, blob_ref
            )));
        }
    }
    Ok(())
}
pub(super) async fn validate_vfs_manifest_blob_refs(
    store: &dyn BlobStore,
    manifest: &vfs::VfsSnapshotManifest,
) -> Result<(), AgentApiError> {
    let mut refs = BTreeMap::new();
    collect_vfs_manifest_blob_refs(&manifest.root, &mut refs)?;
    for (blob_ref, expected_bytes) in refs {
        let info = store
            .stat_blob(&blob_ref)
            .await
            .map_err(map_vfs_manifest_blob_error)?;
        if info.byte_len != expected_bytes {
            return Err(AgentApiError::invalid_request(format!(
                "vfs manifest file size for {blob_ref} is {expected_bytes}, but stored blob size is {}",
                info.byte_len
            )));
        }
    }
    Ok(())
}

pub(super) fn collect_vfs_manifest_blob_refs(
    directory: &vfs::VfsDirectory,
    refs: &mut BTreeMap<BlobRef, u64>,
) -> Result<(), AgentApiError> {
    for entry in directory.entries.values() {
        match entry {
            vfs::VfsEntry::File(file) => {
                if let Some(existing) = refs.insert(file.blob_ref.clone(), file.size_bytes)
                    && existing != file.size_bytes
                {
                    return Err(AgentApiError::invalid_request(format!(
                        "vfs manifest references blob {} with conflicting sizes: {existing} and {}",
                        file.blob_ref, file.size_bytes
                    )));
                }
            }
            vfs::VfsEntry::Directory(directory) => {
                collect_vfs_manifest_blob_refs(directory, refs)?;
            }
        }
    }
    Ok(())
}
pub(super) fn now_ms() -> Result<i64, AgentApiError> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AgentApiError::internal(format!("system clock is before epoch: {error}")))?
        .as_millis();
    i64::try_from(ms)
        .map_err(|_| AgentApiError::internal("current timestamp does not fit in i64 milliseconds"))
}
