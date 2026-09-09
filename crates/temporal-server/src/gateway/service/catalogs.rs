use super::*;

impl GatewayAgentApi {
    pub(super) async fn apply_catalog_refresh_commands(
        &self,
        session_id: &SessionId,
        commands: Vec<CoreAgentCommand>,
    ) -> Result<(), AgentApiError> {
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
}

/// Public context edits cannot write or remove runtime-owned slots.
pub(super) fn parse_client_context_key(value: String) -> Result<ContextEntryKey, AgentApiError> {
    let key = ContextEntryKey::try_new(value)
        .map_err(|error| AgentApiError::invalid_request(format!("invalid context key: {error}")))?;
    if key.as_str() == "runtime" || key.as_str().starts_with("runtime.") {
        return Err(AgentApiError::invalid_request(
            "runtime context keys are owned by the runtime",
        ));
    }
    engine::validate_external_context_key(&key)
        .map_err(|error| AgentApiError::invalid_request(format!("invalid context key: {error}")))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_context_keys_reserve_runtime_namespaces() {
        for key in [
            "runtime",
            "runtime.catalog",
            VFS_CATALOG_CONTEXT_KEY,
            SKILL_CATALOG_CONTEXT_KEY,
            SUBAGENT_CATALOG_CONTEXT_KEY,
            "runtime.catalog.skills.environment",
            "run",
            "run.1",
        ] {
            let error = parse_client_context_key(key.to_owned()).expect_err(key);
            assert_eq!(error.kind, AgentApiErrorKind::InvalidRequest);
        }
        for key in [
            "bot:directory",
            "client.catalog",
            "runtime-info",
            "runtime_extra",
            "instructions.client",
        ] {
            assert_eq!(
                parse_client_context_key(key.to_owned()).unwrap().as_str(),
                key
            );
        }
    }
}
