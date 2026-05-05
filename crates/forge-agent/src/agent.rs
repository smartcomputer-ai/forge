//! Reusable agent definition and version records.
//!
//! Agent versions are immutable configuration bundles that sessions reference.

use crate::config::RunConfig;
use crate::ids::{AgentId, AgentVersionId};
use crate::refs::ArtifactRef;
use crate::tooling::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Stable identity for the reusable agent definition.
    pub agent_id: AgentId,
    /// Human/stable routing handle for this agent, independent of versioned
    /// display or prompt-facing text.
    pub handle: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentVersion {
    /// Immutable version id for a prompt/config/tool bundle.
    pub agent_version_id: AgentVersionId,
    pub agent_id: AgentId,
    /// Versioned display or prompt-facing name.
    pub name: String,
    /// Versioned display or prompt-facing description.
    pub description: Option<String>,
    pub system_prompt_refs: Vec<ArtifactRef>,
    pub developer_prompt_refs: Vec<ArtifactRef>,
    pub default_run_config: RunConfig,
    pub tool_registry: ToolRegistry,
    pub default_tool_profile: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunConfig;
    use crate::refs::{ArtifactKind, ArtifactRef};

    #[test]
    fn agent_definition_round_trips_with_stable_handle() {
        let definition = AgentDefinition {
            agent_id: AgentId::new("agent"),
            handle: "builder".into(),
        };

        let encoded = serde_json::to_string(&definition).expect("serialize agent definition");
        let decoded: AgentDefinition =
            serde_json::from_str(&encoded).expect("deserialize agent definition");

        assert_eq!(decoded, definition);
    }

    #[test]
    fn agent_version_round_trips_through_json() {
        let version = AgentVersion {
            agent_version_id: AgentVersionId::new("agent-v1"),
            agent_id: AgentId::new("agent"),
            name: "Builder".into(),
            description: Some("Builds software from specs".into()),
            system_prompt_refs: vec![ArtifactRef::new("blob://system", ArtifactKind::UserPrompt)],
            default_run_config: RunConfig {
                provider: "openai".into(),
                model: "gpt-x".into(),
                ..Default::default()
            },
            default_tool_profile: Some("default".into()),
            ..Default::default()
        };

        let encoded = serde_json::to_string(&version).expect("serialize agent version");
        let decoded: AgentVersion =
            serde_json::from_str(&encoded).expect("deserialize agent version");

        assert_eq!(decoded, version);
    }
}
