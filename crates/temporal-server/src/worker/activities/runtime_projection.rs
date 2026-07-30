use engine::{
    BlobRef, ContextEntry, ContextEntryId, ContextEntryKey, ContextEntryKind, ContextEntrySource,
    CoreAgentCommand, CoreAgentState, ENVIRONMENT_ACTIVE_CONTEXT_KEY,
    ENVIRONMENT_CATALOG_CONTEXT_KEY, SKILL_CATALOG_CONTEXT_KEY, VFS_CATALOG_CONTEXT_KEY,
};
use temporal_workflow::{
    RuntimeProjectionRefreshActivityRequest, RuntimeProjectionRefreshActivityResult,
};
use temporalio_sdk::activities::ActivityError;
use tools::{
    environment::EnvironmentToolContext,
    prompts::{
        PromptAssemblyLimits, configured_vfs_prompt_root_specs,
        prepare_prompt_instructions_publication_with_warnings, resolve_linked_vfs_prompt_roots,
    },
    skills::{
        configured_vfs_skill_root_specs, prepare_skill_catalog_publication_with_warnings,
        resolve_linked_vfs_skill_roots, skill_catalog_context_input,
    },
    targets::ENV_TARGET_NAMESPACE,
};

use crate::environment::{SessionEnvironmentManager, runtime_environment_from_binding_record};

use super::{common::activity_error, state::RuntimeProjectionActivityDeps};

pub(super) async fn refresh_runtime_projection(
    deps: Option<&RuntimeProjectionActivityDeps>,
    request: RuntimeProjectionRefreshActivityRequest,
) -> Result<RuntimeProjectionRefreshActivityResult, ActivityError> {
    let Some(deps) = deps else {
        return Ok(RuntimeProjectionRefreshActivityResult {
            commands: Vec::new(),
        });
    };

    let links = vfs::resolve_workspace_links(
        deps.blobs.clone(),
        deps.workspace_store.clone(),
        &request.workspace_links,
    )
    .await
    .map_err(activity_error)?;
    let mut state = CoreAgentState::new();
    if let Some(catalog_ref) = request.active_catalog_ref.clone() {
        state
            .context
            .entries
            .push(active_catalog_entry(catalog_ref));
    }
    if let Some(catalog_ref) = request.active_vfs_catalog_ref.clone() {
        state
            .context
            .entries
            .push(active_vfs_catalog_entry(catalog_ref));
    }
    if let Some(catalog_ref) = request.active_environment_catalog_ref.clone() {
        state
            .context
            .entries
            .push(active_environment_catalog_entry(catalog_ref));
    }
    if let Some(active_ref) = request.active_environment_active_ref.clone() {
        state
            .context
            .entries
            .push(active_environment_active_entry(active_ref));
    }
    if let Some(target) = request.active_environment_target.clone() {
        state
            .tooling
            .routing
            .default_targets
            .insert(ENV_TARGET_NAMESPACE.to_owned(), target);
    }

    let bindings = ::environments::SessionEnvironmentBindingStore::list_bindings_for_session(
        deps.environment_bindings.as_ref(),
        &request.session_id,
    )
    .await
    .map_err(activity_error)?
    .into_iter();
    let mut environments = Vec::new();
    for binding in bindings {
        let instance = ::environments::EnvironmentInstanceStore::read_instance(
            deps.environment_instances.as_ref(),
            &binding.instance_id,
        )
        .await
        .map_err(activity_error)?;
        let tool_context = EnvironmentToolContext::new(None, deps.blobs.clone())
            .with_session_id(binding.session_id.as_str());
        environments.push(
            runtime_environment_from_binding_record(&binding, &instance, tool_context)
                .map_err(activity_error)?,
        );
    }
    let manager = SessionEnvironmentManager::new(deps.blobs.clone());
    let mut commands = manager
        .refresh_projection_for_runtime_environments(
            &state,
            links.clone(),
            environments,
            request.vfs_catalog_enabled,
            request.environment_catalog_enabled,
        )
        .await
        .map(|refresh| refresh.commands)
        .map_err(activity_error)?;

    let prompt_entries = if request.vfs_prompts_enabled {
        let specs = configured_vfs_prompt_root_specs(&links, request.vfs_prompt_roots.as_deref())
            .map_err(activity_error)?;
        let resolved = resolve_linked_vfs_prompt_roots(
            deps.blobs.clone(),
            deps.workspace_store.clone(),
            links.clone(),
            specs,
        )
        .await
        .map_err(activity_error)?;
        let inputs = resolved
            .existing_directory_inputs()
            .await
            .map_err(activity_error)?;
        prepare_prompt_instructions_publication_with_warnings(
            deps.blobs.as_ref(),
            &inputs,
            PromptAssemblyLimits::default(),
            resolved.warnings().to_vec(),
        )
        .await
        .map_err(activity_error)?
        .desired
    } else {
        Default::default()
    };
    let desired_instructions = replace_prompt_instruction_source(
        request.active_instruction_inputs.clone(),
        prompt_entries,
        deps.blobs.as_ref(),
    )
    .await
    .map_err(activity_error)?;
    if desired_instructions != request.active_instruction_inputs {
        commands.push(CoreAgentCommand::ReplaceContextPrefix {
            expected_revision: None,
            key_prefix: ContextEntryKey::new("instructions"),
            entries: desired_instructions,
        });
    }

    if !request.vfs_skills_enabled {
        return Ok(RuntimeProjectionRefreshActivityResult {
            commands: append_optional(
                commands,
                clear_catalog_command(request.active_catalog_ref.as_ref()),
            ),
        });
    }
    let specs = configured_vfs_skill_root_specs(&links, request.vfs_skill_roots.as_deref())
        .map_err(activity_error)?;
    if specs.is_empty() {
        return Ok(RuntimeProjectionRefreshActivityResult {
            commands: append_optional(
                commands,
                clear_catalog_command(request.active_catalog_ref.as_ref()),
            ),
        });
    }

    let resolved = resolve_linked_vfs_skill_roots(
        deps.blobs.clone(),
        deps.workspace_store.clone(),
        links,
        specs,
    )
    .await
    .map_err(activity_error)?;
    let inputs = resolved
        .existing_directory_inputs()
        .await
        .map_err(activity_error)?;
    if inputs.is_empty() && resolved.warnings().is_empty() {
        return Ok(RuntimeProjectionRefreshActivityResult {
            commands: append_optional(
                commands,
                clear_catalog_command(request.active_catalog_ref.as_ref()),
            ),
        });
    }

    let publication = prepare_skill_catalog_publication_with_warnings(
        deps.blobs.as_ref(),
        &state,
        None,
        &inputs,
        resolved.warnings().to_vec(),
    )
    .await
    .map_err(activity_error)?;
    if let Some(command) = publication.command {
        commands.push(command);
    }
    Ok(RuntimeProjectionRefreshActivityResult { commands })
}

async fn replace_prompt_instruction_source(
    mut active: std::collections::BTreeMap<ContextEntryKey, engine::ContextEntryInput>,
    prompts: std::collections::BTreeMap<ContextEntryKey, engine::ContextEntryInput>,
    blobs: &dyn engine::storage::BlobStore,
) -> Result<
    std::collections::BTreeMap<ContextEntryKey, engine::ContextEntryInput>,
    engine::storage::BlobStoreError,
> {
    active.retain(|key, _| {
        !(key.as_str() == tools::prompts::PROMPT_INSTRUCTIONS_CONTEXT_KEY_PREFIX
            || key.as_str().starts_with(&format!(
                "{}.",
                tools::prompts::PROMPT_INSTRUCTIONS_CONTEXT_KEY_PREFIX
            )))
    });
    active.extend(prompts);
    let default_key = ContextEntryKey::new("instructions.000.default");
    active.remove(&default_key);
    if active.is_empty() {
        let content_ref = blobs
            .put_bytes(
                temporal_workflow::default_instructions()
                    .as_bytes()
                    .to_vec(),
            )
            .await?;
        active.insert(
            default_key,
            engine::ContextEntryInput {
                kind: ContextEntryKind::Instructions,
                content_ref,
                media_type: Some("text/plain".to_owned()),
                preview: None,
                provider_kind: None,
                provider_item_id: None,
                token_estimate: None,
            },
        );
    }
    Ok(active)
}

fn append_optional(
    mut commands: Vec<CoreAgentCommand>,
    command: Option<CoreAgentCommand>,
) -> Vec<CoreAgentCommand> {
    if let Some(command) = command {
        commands.push(command);
    }
    commands
}

fn clear_catalog_command(active_catalog_ref: Option<&BlobRef>) -> Option<CoreAgentCommand> {
    active_catalog_ref.map(|_| CoreAgentCommand::RemoveContext {
        expected_revision: None,
        key: ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY),
    })
}

fn active_catalog_entry(catalog_ref: BlobRef) -> ContextEntry {
    let input = skill_catalog_context_input(catalog_ref);
    ContextEntry {
        entry_id: ContextEntryId::new(1),
        key: Some(ContextEntryKey::new(SKILL_CATALOG_CONTEXT_KEY)),
        kind: ContextEntryKind::SkillCatalog,
        source: ContextEntrySource::Runtime {
            label: "skills.catalog".to_owned(),
        },
        content_ref: input.content_ref,
        media_type: input.media_type,
        preview: input.preview,
        provider_kind: input.provider_kind,
        provider_item_id: input.provider_item_id,
        token_estimate: input.token_estimate,
    }
}

fn active_vfs_catalog_entry(catalog_ref: BlobRef) -> ContextEntry {
    let input = tools::environment::projection::vfs_catalog_context_input(catalog_ref);
    active_projection_entry(
        ContextEntryKey::new(VFS_CATALOG_CONTEXT_KEY),
        ContextEntryKind::VfsCatalog,
        input,
        "environment.vfs_catalog",
    )
}

fn active_environment_catalog_entry(catalog_ref: BlobRef) -> ContextEntry {
    let input = tools::environment::projection::environment_catalog_context_input(catalog_ref);
    active_projection_entry(
        ContextEntryKey::new(ENVIRONMENT_CATALOG_CONTEXT_KEY),
        ContextEntryKind::EnvironmentCatalog,
        input,
        "environment.catalog",
    )
}

fn active_environment_active_entry(active_ref: BlobRef) -> ContextEntry {
    let input = tools::environment::projection::environment_active_context_input(active_ref);
    active_projection_entry(
        ContextEntryKey::new(ENVIRONMENT_ACTIVE_CONTEXT_KEY),
        ContextEntryKind::EnvironmentActive,
        input,
        "environment.active",
    )
}

fn active_projection_entry(
    key: ContextEntryKey,
    kind: ContextEntryKind,
    input: engine::ContextEntryInput,
    label: &'static str,
) -> ContextEntry {
    ContextEntry {
        entry_id: ContextEntryId::new(1),
        key: Some(key),
        kind,
        source: ContextEntrySource::Runtime {
            label: label.to_owned(),
        },
        content_ref: input.content_ref,
        media_type: input.media_type,
        preview: input.preview,
        provider_kind: input.provider_kind,
        provider_item_id: input.provider_item_id,
        token_estimate: input.token_estimate,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use engine::{
        SessionId, ToolExecutionTarget, WorkspaceLink, WorkspaceLinkAccess, WorkspaceLinkTarget,
        storage::{BlobStore, InMemoryBlobStore},
    };
    use environments::{
        EnvironmentId, EnvironmentInstanceId, EnvironmentInstanceOrigin, EnvironmentInstanceStore,
        EnvironmentProviderCapabilities, EnvironmentProviderId, EnvironmentProviderKind,
        EnvironmentProviderStore, HostControllerConnectionSpec, InMemoryEnvironmentRegistryStore,
        ObserveEnvironmentInstance, PutSessionEnvironmentBinding, RegisterEnvironmentProvider,
        SessionEnvironmentBindingStore, SessionEnvironmentFsRoute, SessionEnvironmentFsRouteAccess,
    };
    use host_protocol::shared::{
        HostCapabilities, HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport,
        ImplementationInfo,
    };
    use tools::environment::projection::{EnvironmentActive, EnvironmentCatalogSnapshot};
    use vfs::{
        CompareAndSetVfsWorkspaceHead, CreateInlineSnapshotRequest, CreateVfsWorkspaceRecord,
        InlineFile, VfsCatalogError, VfsWorkspaceId, VfsWorkspaceRecord, VfsWorkspaceStore,
        create_inline_snapshot,
    };

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_projection_refresh_preserves_bound_active_environment_projection() {
        let blobs: Arc<dyn BlobStore> = Arc::new(InMemoryBlobStore::new());
        let vfs = Arc::new(EmptyVfsStore);
        let bindings = Arc::new(InMemoryEnvironmentRegistryStore::new());
        bindings
            .register_provider(RegisterEnvironmentProvider {
                provider_id: EnvironmentProviderId::new("bridge-local"),
                provider_kind: EnvironmentProviderKind::Bridge,
                display_name: None,
                controller_connection: HostControllerConnectionSpec::new(
                    "ws://127.0.0.1:9001/controller",
                    HostTransport::WebSocket,
                ),
                capabilities: EnvironmentProviderCapabilities {
                    get_target: true,
                    ..EnvironmentProviderCapabilities::default()
                },
                implementation: ImplementationInfo {
                    name: "test".to_owned(),
                    version: None,
                },
                lease_ttl_ms: 60_000,
                metadata: Default::default(),
                observed_at_ms: 1,
            })
            .await
            .expect("create provider");
        bindings
            .observe_instance(test_instance("session_1"))
            .await
            .expect("create instance");
        bindings
            .put_binding(test_binding("session_1", "devbox"))
            .await
            .expect("create binding");
        let deps = RuntimeProjectionActivityDeps {
            blobs: blobs.clone(),
            workspace_store: vfs.clone(),
            environment_bindings: bindings.clone(),
            environment_instances: bindings,
        };

        let result = refresh_runtime_projection(
            Some(&deps),
            RuntimeProjectionRefreshActivityRequest {
                session_id: SessionId::new("session_1"),
                vfs_catalog_enabled: true,
                environment_catalog_enabled: true,
                vfs_prompts_enabled: false,
                vfs_prompt_roots: None,
                active_instruction_inputs: Default::default(),
                vfs_skills_enabled: false,
                vfs_skill_roots: None,
                workspace_links: Vec::new(),
                active_catalog_ref: None,
                active_vfs_catalog_ref: None,
                active_environment_catalog_ref: None,
                active_environment_active_ref: None,
                active_environment_target: Some(ToolExecutionTarget::new("env", "devbox")),
            },
        )
        .await
        .expect("refresh skill catalog");

        let catalog_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                CoreAgentCommand::UpsertContext { key, entry, .. }
                    if key.as_str() == ENVIRONMENT_CATALOG_CONTEXT_KEY =>
                {
                    Some(entry.content_ref.clone())
                }
                _ => None,
            })
            .expect("environment catalog command");
        let catalog: EnvironmentCatalogSnapshot =
            serde_json::from_slice(&blobs.read_bytes(&catalog_ref).await.expect("catalog blob"))
                .expect("catalog json");
        assert_eq!(catalog.active_env_id.as_deref(), Some("devbox"));
        assert_eq!(catalog.environments.len(), 1);
        assert_eq!(catalog.environments[0].env_id, "devbox");
        assert!(catalog.environments[0].capabilities.process_exec);

        let active_ref = result
            .commands
            .iter()
            .find_map(|command| match command {
                CoreAgentCommand::UpsertContext { key, entry, .. }
                    if key.as_str() == ENVIRONMENT_ACTIVE_CONTEXT_KEY =>
                {
                    Some(entry.content_ref.clone())
                }
                _ => None,
            })
            .expect("active environment command");
        let active: EnvironmentActive =
            serde_json::from_slice(&blobs.read_bytes(&active_ref).await.expect("active blob"))
                .expect("active json");
        assert_eq!(active.env_id, "devbox");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_projection_refreshes_prompt_instructions_from_request_links() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let snapshot = create_inline_snapshot(
            blobs.as_ref(),
            CreateInlineSnapshotRequest::new(vec![
                InlineFile::new(
                    ".lightspeed/prompts/instructions.md",
                    b"Use the linked instructions.".to_vec(),
                )
                .unwrap(),
            ]),
        )
        .await
        .unwrap();
        let vfs = Arc::new(EmptyVfsStore);
        let environments = Arc::new(InMemoryEnvironmentRegistryStore::new());
        let deps = RuntimeProjectionActivityDeps {
            blobs: blobs.clone(),
            workspace_store: vfs,
            environment_bindings: environments.clone(),
            environment_instances: environments,
        };

        let result = refresh_runtime_projection(
            Some(&deps),
            RuntimeProjectionRefreshActivityRequest {
                session_id: SessionId::new("session-prompts"),
                workspace_links: vec![WorkspaceLink {
                    path: "/workspace".to_owned(),
                    target: WorkspaceLinkTarget::Snapshot {
                        snapshot_ref: snapshot.snapshot_ref.to_string(),
                    },
                    access: WorkspaceLinkAccess::ReadOnly,
                }],
                vfs_catalog_enabled: false,
                environment_catalog_enabled: false,
                vfs_prompts_enabled: true,
                vfs_prompt_roots: Some(vec!["/workspace/.lightspeed/prompts".to_owned()]),
                active_instruction_inputs: Default::default(),
                vfs_skills_enabled: false,
                vfs_skill_roots: None,
                active_catalog_ref: None,
                active_vfs_catalog_ref: None,
                active_environment_catalog_ref: None,
                active_environment_active_ref: None,
                active_environment_target: None,
            },
        )
        .await
        .expect("refresh runtime projection");

        let entries = result
            .commands
            .iter()
            .find_map(|command| match command {
                CoreAgentCommand::ReplaceContextPrefix { entries, .. } => Some(entries),
                _ => None,
            })
            .expect("prompt context replacement");
        assert_eq!(entries.len(), 1);
        assert!(
            entries
                .keys()
                .next()
                .unwrap()
                .as_str()
                .starts_with(tools::prompts::PROMPT_INSTRUCTIONS_CONTEXT_KEY_PREFIX)
        );
    }

    fn test_binding(session_id: &str, env_id: &str) -> PutSessionEnvironmentBinding {
        PutSessionEnvironmentBinding {
            session_id: SessionId::new(session_id),
            env_id: EnvironmentId::new(env_id),
            instance_id: EnvironmentInstanceId::new("evi-local"),
            cwd: Some(HostPath::new("/workspace").expect("cwd")),
            fs_routes: vec![SessionEnvironmentFsRoute {
                path: HostPath::root(),
                source_path: None,
                access: SessionEnvironmentFsRouteAccess::ReadWrite,
                same_state_as_active_env: Some(EnvironmentId::new(env_id)),
            }],
            updated_at_ms: 1,
        }
    }

    fn test_instance(session_id: &str) -> ObserveEnvironmentInstance {
        ObserveEnvironmentInstance {
            instance_id: EnvironmentInstanceId::new("evi-local"),
            provider_id: EnvironmentProviderId::new("bridge-local"),
            provider_target_id: HostTargetId::new("local-host"),
            origin: EnvironmentInstanceOrigin::Provided,
            display_name: None,
            status: host_protocol::control::targets::HostTargetStatus::Ready,
            scope: HostScope::Session {
                session_id: session_id.to_owned(),
            },
            capabilities: HostCapabilities::filesystem(true, true).with_process(),
            connection: HostConnectionSpec {
                target_id: HostTargetId::new("local-host"),
                endpoint: "ws://127.0.0.1:9001/data".to_owned(),
                transport: HostTransport::WebSocket,
                scope: HostScope::Session {
                    session_id: session_id.to_owned(),
                },
                default_cwd: Some(HostPath::new("/workspace").expect("cwd")),
                capabilities: HostCapabilities::filesystem(true, true).with_process(),
            },
            default_cwd: Some(HostPath::new("/workspace").expect("cwd")),
            metadata: Default::default(),
            observed_at_ms: 1,
        }
    }

    struct EmptyVfsStore;

    #[async_trait]
    impl VfsWorkspaceStore for EmptyVfsStore {
        async fn create_workspace(
            &self,
            record: CreateVfsWorkspaceRecord,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            Ok(VfsWorkspaceRecord {
                workspace_id: record.workspace_id,
                display_name: record.display_name,
                base_snapshot_ref: record.base_snapshot_ref,
                head_snapshot_ref: record.head_snapshot_ref,
                head_totals: record.head_totals,
                revision: 0,
                created_at_ms: record.created_at_ms,
                updated_at_ms: record.created_at_ms,
            })
        }

        async fn read_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            Err(VfsCatalogError::NotFound {
                kind: "workspace",
                id: workspace_id.to_string(),
            })
        }

        async fn list_workspaces(&self) -> Result<Vec<VfsWorkspaceRecord>, VfsCatalogError> {
            Ok(Vec::new())
        }

        async fn compare_and_set_head(
            &self,
            request: CompareAndSetVfsWorkspaceHead,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            Err(VfsCatalogError::NotFound {
                kind: "workspace",
                id: request.workspace_id.to_string(),
            })
        }

        async fn delete_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            Err(VfsCatalogError::NotFound {
                kind: "workspace",
                id: workspace_id.to_string(),
            })
        }
    }
}
