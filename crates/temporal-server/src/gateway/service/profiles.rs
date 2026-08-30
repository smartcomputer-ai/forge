use super::api_config::engine_session_config_from_api;
use super::*;
use ::environments::{EnvironmentProviderBindingStore, EnvironmentStore};
use ::profiles::{ProfileError, ProfileSourceExt, ProfileStore};

const PROFILE_INSTRUCTIONS_CONTEXT_KEY: &str = "instructions.050.profile";

#[derive(Clone, Debug)]
pub(super) struct ResolvedAgentProfile {
    /// Registry identity for named profiles; inline profiles have none.
    pub(super) profile_id: Option<api::ProfileId>,
    pub(super) document: ProfileDocument,
}

impl GatewayAgentApi {
    pub(super) async fn create_profile_record(
        &self,
        params: ProfileCreateParams,
    ) -> Result<ProfileCreateResponse, AgentApiError> {
        let created_at_ms = now_ms()?;
        let profile = self
            .store
            .create_agent_profile(params.profile, created_at_ms)
            .await
            .map_err(map_profile_error)?;
        Ok(ProfileCreateResponse { profile })
    }

    pub(super) async fn read_profile_record(
        &self,
        params: ProfileReadParams,
    ) -> Result<ProfileReadResponse, AgentApiError> {
        let profile = self
            .store
            .read_agent_profile(&params.profile_id)
            .await
            .map_err(map_profile_error)?;
        Ok(ProfileReadResponse { profile })
    }

    pub(super) async fn list_profile_records(
        &self,
        _params: ProfileListParams,
    ) -> Result<ProfileListResponse, AgentApiError> {
        let profiles = self
            .store
            .list_agent_profiles()
            .await
            .map_err(map_profile_error)?;
        Ok(ProfileListResponse { profiles })
    }

    pub(super) async fn put_profile_record(
        &self,
        params: ProfilePutParams,
    ) -> Result<ProfilePutResponse, AgentApiError> {
        let profile = self
            .store
            .put_agent_profile(params.profile, params.expected_revision, now_ms()?)
            .await
            .map_err(map_profile_error)?;
        // Bots pin the profile by id: every open bot on it re-applies the
        // new revision at its controller's next idle boundary.
        self.signal_bots_for_profile(&profile.profile_id).await;
        Ok(ProfilePutResponse { profile })
    }

    pub(super) async fn delete_profile_record(
        &self,
        params: ProfileDeleteParams,
    ) -> Result<ProfileDeleteResponse, AgentApiError> {
        let profile = self
            .store
            .delete_agent_profile(&params.profile_id)
            .await
            .map_err(map_profile_error)?;
        Ok(ProfileDeleteResponse { profile })
    }

    pub(super) async fn apply_profile_to_session(
        &self,
        params: ProfileApplyParams,
    ) -> Result<ProfileApplyResponse, AgentApiError> {
        let session_id = SessionId::try_new(params.session_id).map_err(|error| {
            AgentApiError::invalid_request(format!("invalid session id: {error}"))
        })?;
        let resolved = self.resolve_profile_source(params.profile).await?;
        let (session, applied) = self
            .apply_profile_document(
                &session_id,
                &resolved,
                true,
                params.expected_config_revision,
                params.expected_tools_revision,
            )
            .await?;
        Ok(ProfileApplyResponse { session, applied })
    }

    pub(super) async fn resolve_profile_source(
        &self,
        source: ProfileSource,
    ) -> Result<ResolvedAgentProfile, AgentApiError> {
        source.validate().map_err(map_profile_error)?;
        match source {
            ProfileSource::Named { profile_id } => {
                let profile = self
                    .store
                    .read_agent_profile(&profile_id)
                    .await
                    .map_err(map_profile_error)?;
                Ok(ResolvedAgentProfile {
                    profile_id: Some(profile.profile_id),
                    document: profile.document,
                })
            }
            ProfileSource::Inline { profile } => Ok(ResolvedAgentProfile {
                profile_id: None,
                document: profile.document,
            }),
        }
    }

    pub(super) async fn apply_profile_document(
        &self,
        session_id: &SessionId,
        profile: &ResolvedAgentProfile,
        apply_config: bool,
        expected_config_revision: Option<u64>,
        expected_tools_revision: Option<u64>,
    ) -> Result<(SessionView, ProfileApplySummary), AgentApiError> {
        let document = &profile.document;
        let mut applied = ProfileApplySummary::default();

        if apply_config {
            if let Some(config) = document.config.clone() {
                applied.config_changed = self
                    .apply_profile_config(session_id, config, expected_config_revision)
                    .await?;
            } else if expected_config_revision.is_some() {
                self.assert_config_revision(session_id, expected_config_revision)
                    .await?;
            }
        }

        applied.instructions_changed = self
            .apply_profile_instructions(session_id, document.instructions.clone())
            .await?;

        if expected_tools_revision.is_some() {
            self.assert_tools_revision(session_id, expected_tools_revision)
                .await?;
        }

        match &document.environment {
            None => {}
            Some(ProfileEnvironment::Existing { environment_id }) => {
                applied.active_environment_changed = self
                    .apply_profile_active_environment(session_id, environment_id.clone())
                    .await?;
            }
            Some(ProfileEnvironment::Inherit {}) => {
                let environment_id = self.resolve_inherited_environment(session_id).await?;
                applied.active_environment_changed = self
                    .apply_inherited_environment(session_id, environment_id)
                    .await?;
            }
            Some(ProfileEnvironment::Provision {
                provider_id,
                template_id,
                display_name,
                metadata,
                retention,
                idle_policy,
                credentials,
            }) => {
                let (environment, provisioned) = self
                    .ensure_profile_provisioned_environment(
                        session_id,
                        profile.profile_id.as_ref(),
                        provider_id,
                        template_id,
                        display_name.clone(),
                        metadata.clone(),
                        *retention,
                        idle_policy.clone(),
                    )
                    .await?;
                if provisioned {
                    // Initial credential set for a freshly provisioned
                    // environment (P127 D5): ordinary bindings from here on;
                    // a re-apply that finds the environment does not resync.
                    self.bind_profile_environment_credentials(
                        environment.environment_id.as_str(),
                        credentials,
                    )
                    .await?;
                }
                applied.environment_provisioned = provisioned;
                applied.active_environment_changed = self
                    .apply_profile_active_environment(
                        session_id,
                        environment.environment_id.as_str().to_owned(),
                    )
                    .await?;
            }
        }

        self.load_session_state_with_current_run_context(session_id)
            .await?;
        let session = self.project_session_by_id(session_id).await?;
        Ok((session, applied))
    }

    pub(super) fn merge_profile_start_config(
        &self,
        profile_config: Option<api::SessionConfig>,
        explicit_config: Option<api::SessionConfig>,
    ) -> Option<api::SessionConfig> {
        let Some(profile_config) = profile_config else {
            return explicit_config;
        };
        let Some(explicit_config) = explicit_config else {
            return Some(profile_config);
        };
        Some(api::SessionConfig {
            model: explicit_config.model.or(profile_config.model),
            generation: explicit_config.generation.or(profile_config.generation),
            limits: explicit_config.limits.or(profile_config.limits),
            context: explicit_config.context.or(profile_config.context),
            features: explicit_config.features.or(profile_config.features),
        })
    }

    async fn assert_config_revision(
        &self,
        session_id: &SessionId,
        expected: Option<u64>,
    ) -> Result<(), AgentApiError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        let loaded = self.load_session_state(session_id).await?;
        let actual = loaded.state.lifecycle.config_revision;
        if expected != actual {
            return Err(AgentApiError::conflict(format!(
                "expected config revision {expected}, got {actual}"
            )));
        }
        Ok(())
    }

    async fn assert_tools_revision(
        &self,
        session_id: &SessionId,
        expected: Option<u64>,
    ) -> Result<(), AgentApiError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        let loaded = self.load_session_state(session_id).await?;
        let actual = loaded.state.tooling.revision;
        if expected != actual {
            return Err(AgentApiError::conflict(format!(
                "expected tools revision {expected}, got {actual}"
            )));
        }
        Ok(())
    }

    async fn apply_profile_config(
        &self,
        session_id: &SessionId,
        config: api::SessionConfig,
        expected_revision: Option<u64>,
    ) -> Result<bool, AgentApiError> {
        let loaded = self.load_session_state(session_id).await?;
        self.require_open_idle_session(session_id, &loaded, "profile config apply")?;
        let current = loaded.state.lifecycle.config.as_ref().ok_or_else(|| {
            AgentApiError::invalid_request(format!("session is missing config: {session_id}"))
        })?;
        if let Some(expected) = expected_revision {
            let actual = loaded.state.lifecycle.config_revision;
            if expected != actual {
                return Err(AgentApiError::conflict(format!(
                    "expected config revision {expected}, got {actual}"
                )));
            }
        }
        // Apply means "make the session's config the profile's config":
        // full-document put semantics, sections absent from the profile
        // revert to defaults.
        let candidate = engine_session_config_from_api(config.clone(), self.default_model.clone())?;
        candidate
            .validate()
            .map_err(|error| AgentApiError::invalid_request(error.to_string()))?;
        if &candidate == current {
            return Ok(false);
        }
        self.put_session_config(SessionConfigPutParams {
            session_id: session_id.as_str().to_owned(),
            expected_config_revision: Some(loaded.state.lifecycle.config_revision),
            config,
        })
        .await?;
        Ok(true)
    }

    async fn apply_profile_instructions(
        &self,
        session_id: &SessionId,
        instructions: Option<ProfileInstructions>,
    ) -> Result<bool, AgentApiError> {
        let mut source_entries = BTreeMap::new();
        if let Some(instructions) = instructions {
            let content_ref = match instructions {
                ProfileInstructions::Text { text } => self
                    .store
                    .as_ref()
                    .put_bytes(text.into_bytes())
                    .await
                    .map_err(map_blob_store_error)?,
                ProfileInstructions::TextRef { blob_ref } => {
                    let blob_ref = parse_blob_ref(&blob_ref)?;
                    if !self
                        .store
                        .as_ref()
                        .has_blob(&blob_ref)
                        .await
                        .map_err(map_blob_store_error)?
                    {
                        return Err(AgentApiError::not_found(format!(
                            "profile instructions blob not found: {blob_ref}"
                        )));
                    }
                    blob_ref
                }
            };
            source_entries.insert(
                ContextEntryKey::new(PROFILE_INSTRUCTIONS_CONTEXT_KEY),
                ContextEntryInput {
                    kind: ContextEntryKind::Instructions,
                    content_ref,
                    media_type: Some("text/plain".to_owned()),
                    preview: Some("Profile instructions".to_owned()),
                    provider_kind: None,
                    provider_item_id: None,
                    token_estimate: None,
                },
            );
        }
        let loaded = self.load_session_state(session_id).await?;
        self.require_open_idle_session(session_id, &loaded, "profile instructions apply")?;
        self.reconcile_managed_instructions(
            session_id,
            &loaded.state,
            PROFILE_INSTRUCTIONS_CONTEXT_KEY,
            source_entries,
        )
        .await
    }

    /// Validate that a `provision` profile can be applied in this universe
    /// before any session exists: the provider must have an enabled binding
    /// here. Returns the binding so the applier can create from it.
    pub(super) async fn resolve_profile_provision_binding(
        &self,
        provider_id: &str,
    ) -> Result<::environments::EnvironmentProviderBindingRecord, AgentApiError> {
        let provider_id = parse_environment_provider_id(provider_id.to_owned())?;
        let bindings = EnvironmentProviderBindingStore::list_provider_bindings(
            self.store.as_ref(),
            self.universe_id(),
        )
        .await
        .map_err(map_environments_error)?;
        let binding = bindings
            .into_iter()
            .find(|binding| binding.provider_id == provider_id)
            .ok_or_else(|| {
                AgentApiError::rejected(format!(
                    "profile provisions from environment provider {provider_id}, but this universe has no binding for it"
                ))
            })?;
        if binding.status != ::environments::EnvironmentProviderBindingStatus::Enabled {
            return Err(AgentApiError::rejected(format!(
                "profile provisions from environment provider {provider_id}, but binding {} is disabled",
                binding.binding_id
            )));
        }
        Ok(binding)
    }

    /// Create (or find) the one environment a profile may provision for this
    /// session. The request id is derived from the session id, so retries and
    /// repeated applies converge on the same environment.
    #[allow(clippy::too_many_arguments)]
    async fn ensure_profile_provisioned_environment(
        &self,
        session_id: &SessionId,
        profile_id: Option<&api::ProfileId>,
        provider_id: &str,
        template_id: &str,
        display_name: Option<String>,
        metadata: BTreeMap<String, String>,
        retention: api::ProfileEnvironmentRetention,
        idle_policy: Option<api::EnvironmentIdlePolicyView>,
    ) -> Result<(::environments::EnvironmentRecord, bool), AgentApiError> {
        let request_id = ::environments::EnvironmentProvisionRequestId::for_session(session_id);
        let existing = match EnvironmentStore::read_environment_by_request_id(
            self.store.as_ref(),
            &request_id,
        )
        .await
        {
            Ok(environment) => Some(environment),
            Err(::environments::EnvironmentRegistryError::NotFound { .. }) => None,
            Err(error) => return Err(map_environments_error(error)),
        };
        if let Some(environment) = existing {
            return match environment.status {
                ::environments::EnvironmentStatus::Closing
                | ::environments::EnvironmentStatus::Closed
                | ::environments::EnvironmentStatus::Failed => {
                    Err(AgentApiError::rejected(format!(
                        "the environment provisioned for session {session_id} ({}) is {}; activate another environment or create one through environments/create",
                        environment.environment_id,
                        format!("{:?}", environment.status).to_lowercase()
                    )))
                }
                _ => Ok((environment, false)),
            };
        }
        let binding = self.resolve_profile_provision_binding(provider_id).await?;
        let display_name = display_name.or_else(|| {
            profile_id
                .map(|id| format!("{id} · {session_id}"))
                .or_else(|| Some(format!("session {session_id}")))
        });
        let environment = self
            .create_environment_record_with_origin(
                EnvironmentCreateParams {
                    request_id: request_id.as_str().to_owned(),
                    binding_id: binding.binding_id.as_str().to_owned(),
                    template_id: template_id.to_owned(),
                    display_name,
                    metadata,
                    idle_policy,
                },
                Some(::environments::EnvironmentOriginSession {
                    session_id: session_id.clone(),
                    profile_id: profile_id.map(|id| id.as_str().to_owned()),
                    close_with_session: matches!(
                        retention,
                        api::ProfileEnvironmentRetention::CloseWithSession
                    ),
                }),
            )
            .await?;
        Ok((environment, true))
    }

    /// Bind the profile's requested credentials to a just-provisioned
    /// environment. Each entry goes through the same validation as
    /// `environments/credentials/bind`; a failure surfaces as the apply
    /// error (the session start fails; a `closeWithSession` environment is
    /// cleaned up when the session closes).
    async fn bind_profile_environment_credentials(
        &self,
        environment_id: &str,
        credentials: &[api::ProfileEnvironmentCredential],
    ) -> Result<(), AgentApiError> {
        for credential in credentials {
            self.bind_environment_credential_record(EnvironmentCredentialBindParams {
                environment_id: environment_id.to_owned(),
                env_name: credential.env_name.clone(),
                source: credential.source.clone(),
            })
            .await
            .map_err(|error| {
                AgentApiError::new(
                    error.kind,
                    format!(
                        "profile environment credential {}: {}",
                        credential.env_name, error.message
                    ),
                )
            })?;
        }
        Ok(())
    }

    /// Validate profile credential references against this universe before a
    /// session or environment exists (grant active, provider has a credential,
    /// secret present). Mirrors the bind-time checks so a broken reference
    /// fails at admission with a typed error.
    pub(super) async fn validate_profile_environment_credentials(
        &self,
        credentials: &[api::ProfileEnvironmentCredential],
    ) -> Result<(), AgentApiError> {
        for credential in credentials {
            environment_credentials::validate_credential_env_name(&credential.env_name)?;
            self.credential_source_from_api(credential.source.clone())
                .await
                .map_err(|error| {
                    AgentApiError::new(
                        error.kind,
                        format!(
                            "profile environment credential {}: {}",
                            credential.env_name, error.message
                        ),
                    )
                })?;
        }
        Ok(())
    }

    async fn apply_profile_active_environment(
        &self,
        session_id: &SessionId,
        environment_id: api::EnvironmentId,
    ) -> Result<bool, AgentApiError> {
        let loaded = self.load_session_state(session_id).await?;
        if loaded
            .state
            .environment
            .active_environment_id
            .as_ref()
            .is_some_and(|active| active.as_str() == environment_id)
        {
            return Ok(false);
        }
        self.activate_session_environment(SessionEnvironmentActivateParams {
            session_id: session_id.as_str().to_owned(),
            environment_id,
        })
        .await?;
        Ok(true)
    }
}

pub(super) fn map_profile_error(error: ProfileError) -> AgentApiError {
    match error {
        ProfileError::AlreadyExists { profile_id } => {
            AgentApiError::conflict(format!("agent profile already exists: {profile_id}"))
        }
        ProfileError::NotFound { profile_id } => {
            AgentApiError::not_found(format!("agent profile not found: {profile_id}"))
        }
        ProfileError::RevisionConflict {
            profile_id,
            expected,
            actual,
        } => AgentApiError::conflict(format!(
            "agent profile revision conflict for {profile_id}: expected {expected}, got {actual}"
        )),
        ProfileError::InvalidInput { message } => AgentApiError::invalid_request(message),
        ProfileError::Store { message } => AgentApiError::internal(message),
    }
}
