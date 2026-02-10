use super::{AgentError, SessionError, SessionPersistenceWriter};
use forge_cxdb_runtime::{
    CxdbBinaryClient, CxdbClientError, CxdbFsSnapshotCapture, CxdbFsSnapshotPolicy, CxdbHttpClient,
    CxdbRuntimeStore,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct FsSnapshotStatsRecord {
    pub(super) file_count: usize,
    pub(super) dir_count: usize,
    pub(super) symlink_count: usize,
    pub(super) total_bytes: i64,
    pub(super) bytes_uploaded: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionLifecycleRecord {
    pub(super) session_id: String,
    pub(super) kind: String,
    pub(super) timestamp: String,
    pub(super) final_state: Option<String>,
    pub(super) sequence_no: u64,
    pub(super) thread_key: Option<String>,
    pub(super) fs_root_hash: Option<String>,
    pub(super) snapshot_policy_id: Option<String>,
    pub(super) snapshot_stats: Option<FsSnapshotStatsRecord>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct AgentTurnRecord {
    pub(super) session_id: String,
    pub(super) timestamp: String,
    pub(super) turn: Value,
    pub(super) sequence_no: u64,
    pub(super) thread_key: Option<String>,
    pub(super) fs_root_hash: Option<String>,
    pub(super) snapshot_policy_id: Option<String>,
    pub(super) snapshot_stats: Option<FsSnapshotStatsRecord>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct ToolCallLifecycleRecord {
    pub(super) session_id: String,
    pub(super) kind: String,
    pub(super) timestamp: String,
    pub(super) call_id: String,
    pub(super) tool_name: Option<String>,
    pub(super) arguments: Option<Value>,
    pub(super) output: Option<Value>,
    pub(super) is_error: Option<bool>,
    pub(super) sequence_no: u64,
    pub(super) thread_key: Option<String>,
    pub(super) fs_root_hash: Option<String>,
    pub(super) snapshot_policy_id: Option<String>,
    pub(super) snapshot_stats: Option<FsSnapshotStatsRecord>,
}

pub(super) const AGENT_REGISTRY_BUNDLE_ID: &str = "forge.agent.runtime.v2";
const AGENT_TRANSCRIPT_TYPE_VERSION: u32 = 2;

fn type_field_tags(type_id: &str) -> &'static [(&'static str, &'static str)] {
    const TURN_FIELDS: [(&str, &str); 8] = [
        ("session_id", "1"),
        ("timestamp", "2"),
        ("turn", "3"),
        ("sequence_no", "4"),
        ("thread_key", "5"),
        ("fs_root_hash", "6"),
        ("snapshot_policy_id", "7"),
        ("snapshot_stats", "8"),
    ];
    const SESSION_LIFECYCLE_FIELDS: [(&str, &str); 9] = [
        ("session_id", "1"),
        ("kind", "2"),
        ("timestamp", "3"),
        ("final_state", "4"),
        ("sequence_no", "5"),
        ("thread_key", "6"),
        ("fs_root_hash", "7"),
        ("snapshot_policy_id", "8"),
        ("snapshot_stats", "9"),
    ];
    const TOOL_CALL_LIFECYCLE_FIELDS: [(&str, &str); 13] = [
        ("session_id", "1"),
        ("kind", "2"),
        ("timestamp", "3"),
        ("call_id", "4"),
        ("tool_name", "5"),
        ("arguments", "6"),
        ("output", "7"),
        ("is_error", "8"),
        ("sequence_no", "9"),
        ("thread_key", "10"),
        ("fs_root_hash", "11"),
        ("snapshot_policy_id", "12"),
        ("snapshot_stats", "13"),
    ];
    match type_id {
        "forge.agent.user_turn"
        | "forge.agent.assistant_turn"
        | "forge.agent.tool_results_turn"
        | "forge.agent.system_turn"
        | "forge.agent.steering_turn"
        | "forge.link.subagent_spawn" => &TURN_FIELDS,
        "forge.agent.session_lifecycle" => &SESSION_LIFECYCLE_FIELDS,
        "forge.agent.tool_call_lifecycle" => &TOOL_CALL_LIFECYCLE_FIELDS,
        _ => &[],
    }
}

pub(super) fn encode_typed_record<T: Serialize>(
    type_id: &str,
    record: &T,
) -> Result<Vec<u8>, SessionError> {
    let value = serde_json::to_value(record)
        .map_err(|err| SessionError::Persistence(format!("json encode failed: {err}")))?;

    let Some(object) = value.as_object() else {
        return rmp_serde::to_vec_named(record)
            .map_err(|err| SessionError::Persistence(format!("msgpack encode failed: {err}")));
    };

    let mut encoded = object.clone();
    for (field_name, tag) in type_field_tags(type_id) {
        if let Some(field_value) = object.get(*field_name) {
            encoded.insert((*tag).to_string(), field_value.clone());
        }
    }

    rmp_serde::to_vec_named(&encoded)
        .map_err(|err| SessionError::Persistence(format!("msgpack encode failed: {err}")))
}

#[allow(dead_code)]
pub(super) fn decode_typed_record<T: DeserializeOwned>(payload: &[u8]) -> Result<T, SessionError> {
    if let Ok(projected) = serde_json::from_slice::<T>(payload) {
        return Ok(projected);
    }
    rmp_serde::from_slice(payload)
        .map_err(|err| SessionError::Persistence(format!("msgpack decode failed: {err}")))
}

pub(super) fn capture_fs_snapshot_blocking(
    store: Arc<dyn SessionPersistenceWriter>,
    policy: Option<&CxdbFsSnapshotPolicy>,
    workspace_root: &Path,
) -> Result<Option<CxdbFsSnapshotCapture>, CxdbClientError> {
    let Some(policy) = policy.cloned() else {
        return Ok(None);
    };
    let workspace_root = workspace_root.to_path_buf();
    run_cxdb_future_blocking("capture_upload_workspace", async move {
        store
            .capture_upload_workspace(&workspace_root, &policy)
            .await
    })
    .map(Some)
}

pub(super) fn snapshot_capture_fields(
    capture: Option<&CxdbFsSnapshotCapture>,
) -> (
    Option<String>,
    Option<String>,
    Option<FsSnapshotStatsRecord>,
) {
    let Some(capture) = capture else {
        return (None, None, None);
    };
    (
        Some(capture.fs_root_hash.clone()),
        Some(capture.policy_id.clone()),
        Some(FsSnapshotStatsRecord {
            file_count: capture.stats.file_count,
            dir_count: capture.stats.dir_count,
            symlink_count: capture.stats.symlink_count,
            total_bytes: capture.stats.total_bytes as i64,
            bytes_uploaded: capture.stats.bytes_uploaded,
        }),
    )
}

pub(super) fn apply_sequence_and_fs_to_record<T: Serialize + DeserializeOwned>(
    record: &mut T,
    sequence_no: u64,
    thread_key: Option<String>,
    capture: Option<&CxdbFsSnapshotCapture>,
) -> Result<(), AgentError> {
    let mut value = serde_json::to_value(&*record).map_err(|error| {
        SessionError::Persistence(format!("failed to serialize record: {error}"))
    })?;
    if !value.is_object() {
        return Err(SessionError::Persistence(
            "typed record should serialize as object".to_string(),
        )
        .into());
    }
    let (fs_root_hash, snapshot_policy_id, snapshot_stats) = snapshot_capture_fields(capture);
    if let Some(object) = value.as_object_mut() {
        object.insert("sequence_no".to_string(), Value::Number(sequence_no.into()));
        object.insert(
            "thread_key".to_string(),
            thread_key.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "fs_root_hash".to_string(),
            fs_root_hash.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "snapshot_policy_id".to_string(),
            snapshot_policy_id.map(Value::String).unwrap_or(Value::Null),
        );
        object.insert(
            "snapshot_stats".to_string(),
            match snapshot_stats {
                Some(stats) => serde_json::to_value(stats).map_err(|error| {
                    SessionError::Persistence(format!("failed to encode snapshot stats: {error}"))
                })?,
                None => Value::Null,
            },
        );
    }
    *record = serde_json::from_value(value).map_err(|error| {
        SessionError::Persistence(format!("failed to hydrate typed record: {error}"))
    })?;
    Ok(())
}

fn encode_idempotency_part(part: &str) -> String {
    format!("{}:{}", part.len(), part)
}

pub(super) fn agent_idempotency_key(
    session_id: &str,
    local_turn_index: u64,
    event_kind: &str,
) -> String {
    format!(
        "forge-agent:v1|{}|{}|{}",
        encode_idempotency_part(session_id),
        local_turn_index,
        encode_idempotency_part(event_kind)
    )
}

pub(super) fn agent_type_version(type_id: &str) -> u32 {
    match type_id {
        "forge.agent.user_turn"
        | "forge.agent.assistant_turn"
        | "forge.agent.tool_results_turn"
        | "forge.agent.system_turn"
        | "forge.agent.steering_turn" => AGENT_TRANSCRIPT_TYPE_VERSION,
        _ => 1,
    }
}

pub(super) fn run_cxdb_future_blocking<F, T>(
    operation: &str,
    future: F,
) -> Result<T, CxdbClientError>
where
    F: std::future::Future<Output = Result<T, CxdbClientError>> + Send + 'static,
    T: Send + 'static,
{
    let operation_name = operation.to_string();
    let spawn_operation = operation_name.clone();
    std::thread::Builder::new()
        .name(format!("forge-agent-{operation_name}"))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    CxdbClientError::Backend(format!(
                        "{spawn_operation} runtime initialization failed: {error}"
                    ))
                })?;
            runtime.block_on(future)
        })
        .map_err(|error| {
            CxdbClientError::Backend(format!("{operation_name} thread spawn failed: {error}"))
        })?
        .join()
        .map_err(|_| CxdbClientError::Backend(format!("{operation_name} task panicked")))?
}

pub(super) fn publish_agent_registry_bundle_blocking(
    store: Arc<CxdbRuntimeStore<Arc<dyn CxdbBinaryClient>, Arc<dyn CxdbHttpClient>>>,
) -> Result<(), AgentError> {
    let bundle_json = agent_registry_bundle_json()?;
    run_cxdb_future_blocking("publish_registry_bundle", async move {
        store
            .publish_registry_bundle(AGENT_REGISTRY_BUNDLE_ID, &bundle_json)
            .await
    })
    .map_err(|error| {
        SessionError::Persistence(format!(
            "publish_registry_bundle failed for '{}': {}",
            AGENT_REGISTRY_BUNDLE_ID, error
        ))
        .into()
    })
}

fn agent_registry_bundle_json() -> Result<Vec<u8>, AgentError> {
    let bundle = serde_json::json!({
        "registry_version": 1,
        "bundle_id": AGENT_REGISTRY_BUNDLE_ID,
        "types": {
            "forge.agent.user_turn": { "versions": { "2": { "fields": turn_fields_descriptor() } } },
            "forge.agent.assistant_turn": { "versions": { "2": { "fields": turn_fields_descriptor() } } },
            "forge.agent.tool_results_turn": { "versions": { "2": { "fields": turn_fields_descriptor() } } },
            "forge.agent.system_turn": { "versions": { "2": { "fields": turn_fields_descriptor() } } },
            "forge.agent.steering_turn": { "versions": { "2": { "fields": turn_fields_descriptor() } } },
            "forge.agent.session_lifecycle": { "versions": { "1": { "fields": session_lifecycle_fields_descriptor() } } },
            "forge.agent.tool_call_lifecycle": { "versions": { "1": { "fields": tool_call_lifecycle_fields_descriptor() } } }
        }
    });
    serde_json::to_vec(&bundle).map_err(|error| {
        SessionError::Persistence(format!(
            "failed to serialize agent registry bundle: {error}"
        ))
        .into()
    })
}

fn turn_fields_descriptor() -> serde_json::Value {
    serde_json::json!({
        "1": { "name": "session_id", "type": "string" },
        "2": { "name": "timestamp", "type": "string" },
        "3": { "name": "turn", "type": "any" },
        "4": { "name": "sequence_no", "type": "u64" },
        "5": { "name": "thread_key", "type": "string", "optional": true },
        "6": { "name": "fs_root_hash", "type": "string", "optional": true },
        "7": { "name": "snapshot_policy_id", "type": "string", "optional": true },
        "8": { "name": "snapshot_stats", "type": "any", "optional": true }
    })
}

fn session_lifecycle_fields_descriptor() -> serde_json::Value {
    serde_json::json!({
        "1": { "name": "session_id", "type": "string" },
        "2": { "name": "kind", "type": "string" },
        "3": { "name": "timestamp", "type": "string" },
        "4": { "name": "final_state", "type": "string", "optional": true },
        "5": { "name": "sequence_no", "type": "u64" },
        "6": { "name": "thread_key", "type": "string", "optional": true },
        "7": { "name": "fs_root_hash", "type": "string", "optional": true },
        "8": { "name": "snapshot_policy_id", "type": "string", "optional": true },
        "9": { "name": "snapshot_stats", "type": "any", "optional": true }
    })
}

fn tool_call_lifecycle_fields_descriptor() -> serde_json::Value {
    serde_json::json!({
        "1": { "name": "session_id", "type": "string" },
        "2": { "name": "kind", "type": "string" },
        "3": { "name": "timestamp", "type": "string" },
        "4": { "name": "call_id", "type": "string" },
        "5": { "name": "tool_name", "type": "string", "optional": true },
        "6": { "name": "arguments", "type": "any", "optional": true },
        "7": { "name": "output", "type": "any", "optional": true },
        "8": { "name": "is_error", "type": "bool", "optional": true },
        "9": { "name": "sequence_no", "type": "u64" },
        "10": { "name": "thread_key", "type": "string", "optional": true },
        "11": { "name": "fs_root_hash", "type": "string", "optional": true },
        "12": { "name": "snapshot_policy_id", "type": "string", "optional": true },
        "13": { "name": "snapshot_stats", "type": "any", "optional": true }
    })
}
