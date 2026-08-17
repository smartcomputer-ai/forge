//! Controller-plane target lifecycle payloads.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::shared::{EnvironmentCapabilities, EnvironmentPath, EnvironmentScope, ProviderTargetId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBindingContext {
    pub universe_id: String,
    pub binding_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentTemplate {
    pub template_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub public_ingress: bool,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTemplatesParams {
    pub binding: ProviderBindingContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTemplatesResponse {
    pub templates: Vec<EnvironmentTemplate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTargetsParams {
    pub binding: ProviderBindingContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ProviderTargetStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTargetsResponse {
    pub targets: Vec<ProviderTargetSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetParams {
    pub request_id: String,
    pub environment_id: String,
    pub incarnation_id: String,
    pub binding: ProviderBindingContext,
    pub template_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetResponse {
    pub target: ProviderTargetSummary,
}

/// Explicit ownership transfer of an existing provider target into a
/// Lightspeed-managed binding namespace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptTargetParams {
    pub request_id: String,
    pub environment_id: String,
    pub incarnation_id: String,
    pub binding: ProviderBindingContext,
    /// Provider-native reference to the existing target. Incus uses
    /// `<project>/<instance>` and defaults the project to `default`.
    pub source_target: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptTargetResponse {
    pub target: ProviderTargetSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTargetParams {
    pub binding: ProviderBindingContext,
    pub target_id: ProviderTargetId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTargetResponse {
    pub target: ProviderTargetSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTargetParams {
    pub request_id: String,
    pub environment_id: String,
    pub incarnation_id: String,
    pub binding: ProviderBindingContext,
    pub target_id: ProviderTargetId,
    #[serde(default)]
    pub force: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTargetResponse {
    pub target_id: ProviderTargetId,
    pub status: ProviderTargetStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxTargetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<EnvironmentPath>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTargetSummary {
    pub target_id: ProviderTargetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub status: ProviderTargetStatus,
    pub scope: EnvironmentScope,
    pub capabilities: EnvironmentCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_cwd: Option<EnvironmentPath>,
    /// Power states this target can be moved to with `controller/setTargetPower`.
    /// Empty when the provider offers no power control beyond create/close.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub power_states: Vec<PowerState>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderTargetStatus {
    Creating,
    Starting,
    Ready,
    /// VMM alive with execution frozen; RAM stays resident, resume is instant.
    Paused,
    /// Execution state saved to disk and the VMM released; resume restores it.
    Suspended,
    Stopped,
    Closing,
    Closed,
    Failed,
    Unknown,
}

impl ProviderTargetStatus {
    /// The power state a steady-state observation corresponds to, or `None`
    /// while the target is transitioning or gone.
    pub fn power_state(self) -> Option<PowerState> {
        match self {
            Self::Ready => Some(PowerState::Running),
            Self::Paused => Some(PowerState::Paused),
            Self::Suspended => Some(PowerState::Suspended),
            Self::Stopped => Some(PowerState::Stopped),
            Self::Creating
            | Self::Starting
            | Self::Closing
            | Self::Closed
            | Self::Failed
            | Self::Unknown => None,
        }
    }
}

/// Requested steady power state of a provider target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PowerState {
    Running,
    Paused,
    Suspended,
    Stopped,
}

impl PowerState {
    pub const ALL: [PowerState; 4] = [
        PowerState::Running,
        PowerState::Paused,
        PowerState::Suspended,
        PowerState::Stopped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Suspended => "suspended",
            Self::Stopped => "stopped",
        }
    }
}

impl std::fmt::Display for PowerState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Converge one target to a steady power state. Idempotent by inventory: a
/// target already in `power` (or transitioning toward it) is reported as
/// observed without further action; resuming may replace the underlying
/// host process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTargetPowerParams {
    pub request_id: String,
    pub environment_id: String,
    pub incarnation_id: String,
    pub binding: ProviderBindingContext,
    pub target_id: ProviderTargetId,
    pub power: PowerState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTargetPowerResponse {
    pub target: ProviderTargetSummary,
}
