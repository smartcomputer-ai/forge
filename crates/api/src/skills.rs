use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillListParams {
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResponse {
    pub catalogs: Vec<SkillCatalogView>,
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

/// Readable paths within the owning catalog's filesystem domain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillLocationView {
    pub skill_dir_path: String,
    pub skill_doc_path: String,
}

/// An independent catalog instance; sources are never merged or used as fallbacks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogView {
    pub source: SkillCatalogSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_ref: Option<String>,
    pub availability: SkillCatalogAvailability,
    pub skills: Vec<SkillListItem>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SkillCatalogSource {
    Vfs,
    Environment { environment_id: EnvironmentId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SkillCatalogAvailability {
    Available,
    Stale,
    Unavailable,
}
