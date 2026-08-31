use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunStartParams {
    pub session_id: SessionId,
    pub source: RunStartSource,
    /// Client-supplied idempotency key, unique per session. Retrying
    /// `session/runs/start` with the same submission id and the same
    /// source/config/terminal notification returns the original run instead
    /// of starting a second one; reusing a submission id with any of those
    /// inputs changed is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<RunStartConfig>,
    /// Request an at-least-once terminal emission to the managed session's
    /// immutable lifecycle controller. The destination is derived by the
    /// gateway; callers supply only the controller's opaque dedup token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_on_terminal: Option<RunTerminalNotificationInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunTerminalNotificationInput {
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RunStartSource {
    Input { items: Vec<InputItem> },
    Context { keys: Vec<String> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunStartConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GenerationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<RunLimitsConfig>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunLimitsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunStartResponse {
    pub run: RunView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunCancelParams {
    pub session_id: SessionId,
    pub run_id: RunId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunCancelResponse {
    pub run: RunView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunApprovalsDecideParams {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub decisions: Vec<ApprovalDecisionInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionInput {
    pub approval_id: String,
    pub decision: ApprovalDecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecisionKind {
    Approve,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunApprovalsDecideResponse {
    pub results: Vec<ApprovalDecisionResult>,
    pub run: RunView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionResult {
    pub approval_id: String,
    pub status: ApprovalDecisionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<ApprovalDecisionFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecisionStatus {
    Decided,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionFailure {
    pub kind: ApprovalDecisionFailureKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecisionFailureKind {
    InvalidId,
    Unknown,
    ForeignRun,
    AlreadyDecided,
    Cancelled,
    Duplicate,
    InvalidNote,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSteerParams {
    pub session_id: SessionId,
    /// The run to steer. Must be the session's current active run (running
    /// or parked on an await); a finished or cancelling run is rejected so a
    /// late steer never lands on the next run.
    pub run_id: RunId,
    /// Steering input, same vocabulary as run input. Delivered to the model
    /// at the next turn boundary of the run; it does not interrupt the
    /// in-flight turn or wake a parked await.
    pub items: Vec<InputItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunSteerResponse {
    /// Identifier of the accepted steering batch within the session.
    pub steering_id: String,
    pub run: RunView,
}
