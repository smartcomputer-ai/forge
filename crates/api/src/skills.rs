use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillListParams {
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResponse {
    /// Independent selected-environment catalog. The top-level fields describe VFS only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSkillCatalogView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_ref: Option<String>,
    #[serde(default)]
    pub skills: Vec<SkillListItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillListItem {
    pub skill_id: SkillId,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub enabled: bool,
    /// Where to read the instructions and resolve supporting files.
    pub location: SkillLocationView,
}

/// The filesystem domain and paths needed for ordinary skill use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkillLocationView {
    Environment {
        environment_id: EnvironmentId,
        skill_dir_path: String,
        skill_doc_path: String,
    },
    Vfs {
        skill_dir_path: String,
        skill_doc_path: String,
    },
}

/// Independently identified catalog, never merged or deduplicated with VFS skills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSkillCatalogView {
    pub catalog_id: String,
    pub context_key: String,
    pub environment_id: EnvironmentId,
    pub catalog_ref: String,
    pub availability: EnvironmentSkillAvailabilityView,
    pub skills: Vec<SkillListItem>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentSkillAvailabilityView {
    Available,
    Stale,
    Unavailable,
}
