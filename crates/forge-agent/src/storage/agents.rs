//! Agent definition and version storage contract.

use crate::agent::{AgentDefinition, AgentVersion};
use crate::ids::{AgentId, AgentVersionId};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentDefinitionStoreError {
    #[error("agent definition store failure: {message}")]
    Store { message: String },

    #[error("agent definition already exists: {agent_id}")]
    DefinitionExists { agent_id: AgentId },

    #[error("agent version already exists: {agent_version_id}")]
    VersionExists { agent_version_id: AgentVersionId },

    #[error("agent version '{agent_version_id}' references unknown agent '{agent_id}'")]
    UnknownAgentForVersion {
        agent_id: AgentId,
        agent_version_id: AgentVersionId,
    },
}

#[async_trait]
pub trait AgentDefinitionStore: Send + Sync {
    async fn put_definition(
        &self,
        definition: AgentDefinition,
    ) -> Result<(), AgentDefinitionStoreError>;

    async fn get_definition(
        &self,
        agent_id: &AgentId,
    ) -> Result<Option<AgentDefinition>, AgentDefinitionStoreError>;

    async fn get_definition_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<AgentDefinition>, AgentDefinitionStoreError>;

    async fn put_version(&self, version: AgentVersion) -> Result<(), AgentDefinitionStoreError>;

    async fn get_version(
        &self,
        agent_version_id: &AgentVersionId,
    ) -> Result<Option<AgentVersion>, AgentDefinitionStoreError>;

    async fn latest_version(
        &self,
        agent_id: &AgentId,
    ) -> Result<Option<AgentVersion>, AgentDefinitionStoreError>;

    async fn list_versions(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<AgentVersion>, AgentDefinitionStoreError>;
}

#[derive(Clone, Default)]
pub struct InMemoryAgentDefinitionStore {
    inner: Arc<RwLock<InMemoryAgentDefinitionStoreInner>>,
}

#[derive(Default)]
struct InMemoryAgentDefinitionStoreInner {
    definitions_by_id: BTreeMap<AgentId, AgentDefinition>,
    definition_ids_by_handle: BTreeMap<String, AgentId>,
    versions_by_id: BTreeMap<AgentVersionId, AgentVersion>,
    version_ids_by_agent_id: BTreeMap<AgentId, Vec<AgentVersionId>>,
}

impl InMemoryAgentDefinitionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentDefinitionStore for InMemoryAgentDefinitionStore {
    async fn put_definition(
        &self,
        definition: AgentDefinition,
    ) -> Result<(), AgentDefinitionStoreError> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition write lock poisoned".into(),
            })?;
        if inner.definitions_by_id.contains_key(&definition.agent_id) {
            return Err(AgentDefinitionStoreError::DefinitionExists {
                agent_id: definition.agent_id,
            });
        }
        inner
            .definition_ids_by_handle
            .insert(definition.handle.clone(), definition.agent_id.clone());
        inner
            .definitions_by_id
            .insert(definition.agent_id.clone(), definition);
        Ok(())
    }

    async fn get_definition(
        &self,
        agent_id: &AgentId,
    ) -> Result<Option<AgentDefinition>, AgentDefinitionStoreError> {
        self.inner
            .read()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition read lock poisoned".into(),
            })
            .map(|inner| inner.definitions_by_id.get(agent_id).cloned())
    }

    async fn get_definition_by_handle(
        &self,
        handle: &str,
    ) -> Result<Option<AgentDefinition>, AgentDefinitionStoreError> {
        let inner = self
            .inner
            .read()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition read lock poisoned".into(),
            })?;
        Ok(inner
            .definition_ids_by_handle
            .get(handle)
            .and_then(|agent_id| inner.definitions_by_id.get(agent_id))
            .cloned())
    }

    async fn put_version(&self, version: AgentVersion) -> Result<(), AgentDefinitionStoreError> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition write lock poisoned".into(),
            })?;
        if !inner.definitions_by_id.contains_key(&version.agent_id) {
            return Err(AgentDefinitionStoreError::UnknownAgentForVersion {
                agent_id: version.agent_id,
                agent_version_id: version.agent_version_id,
            });
        }
        if inner.versions_by_id.contains_key(&version.agent_version_id) {
            return Err(AgentDefinitionStoreError::VersionExists {
                agent_version_id: version.agent_version_id,
            });
        }
        inner
            .version_ids_by_agent_id
            .entry(version.agent_id.clone())
            .or_default()
            .push(version.agent_version_id.clone());
        inner
            .versions_by_id
            .insert(version.agent_version_id.clone(), version);
        Ok(())
    }

    async fn get_version(
        &self,
        agent_version_id: &AgentVersionId,
    ) -> Result<Option<AgentVersion>, AgentDefinitionStoreError> {
        self.inner
            .read()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition read lock poisoned".into(),
            })
            .map(|inner| inner.versions_by_id.get(agent_version_id).cloned())
    }

    async fn latest_version(
        &self,
        agent_id: &AgentId,
    ) -> Result<Option<AgentVersion>, AgentDefinitionStoreError> {
        let inner = self
            .inner
            .read()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition read lock poisoned".into(),
            })?;
        Ok(inner
            .version_ids_by_agent_id
            .get(agent_id)
            .and_then(|version_ids| version_ids.last())
            .and_then(|version_id| inner.versions_by_id.get(version_id))
            .cloned())
    }

    async fn list_versions(
        &self,
        agent_id: &AgentId,
    ) -> Result<Vec<AgentVersion>, AgentDefinitionStoreError> {
        let inner = self
            .inner
            .read()
            .map_err(|_| AgentDefinitionStoreError::Store {
                message: "agent definition read lock poisoned".into(),
            })?;
        Ok(inner
            .version_ids_by_agent_id
            .get(agent_id)
            .into_iter()
            .flat_map(|version_ids| version_ids.iter())
            .filter_map(|version_id| inner.versions_by_id.get(version_id))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> AgentDefinition {
        AgentDefinition {
            agent_id: AgentId::new("agent-a"),
            handle: "builder".into(),
        }
    }

    fn version(id: &str) -> AgentVersion {
        AgentVersion {
            agent_version_id: AgentVersionId::new(id),
            agent_id: AgentId::new("agent-a"),
            name: id.into(),
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_agent_definition_store_reads_definition_by_id_and_handle() {
        let store = InMemoryAgentDefinitionStore::new();
        store
            .put_definition(definition())
            .await
            .expect("put definition");

        assert_eq!(
            store
                .get_definition(&AgentId::new("agent-a"))
                .await
                .expect("get definition")
                .expect("definition")
                .handle,
            "builder"
        );
        assert_eq!(
            store
                .get_definition_by_handle("builder")
                .await
                .expect("get definition by handle")
                .expect("definition")
                .agent_id,
            AgentId::new("agent-a")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_agent_definition_store_preserves_version_order_and_latest() {
        let store = InMemoryAgentDefinitionStore::new();
        store
            .put_definition(definition())
            .await
            .expect("put definition");
        store.put_version(version("v1")).await.expect("put v1");
        store.put_version(version("v2")).await.expect("put v2");

        let versions = store
            .list_versions(&AgentId::new("agent-a"))
            .await
            .expect("list versions");
        assert_eq!(
            versions
                .iter()
                .map(|version| version.agent_version_id.clone())
                .collect::<Vec<_>>(),
            vec![AgentVersionId::new("v1"), AgentVersionId::new("v2")]
        );
        assert_eq!(
            store
                .latest_version(&AgentId::new("agent-a"))
                .await
                .expect("latest version")
                .expect("version")
                .agent_version_id,
            AgentVersionId::new("v2")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_memory_agent_definition_store_rejects_orphan_versions() {
        let store = InMemoryAgentDefinitionStore::new();

        let error = store
            .put_version(version("v1"))
            .await
            .expect_err("orphan version");

        assert!(matches!(
            error,
            AgentDefinitionStoreError::UnknownAgentForVersion { .. }
        ));
    }
}
