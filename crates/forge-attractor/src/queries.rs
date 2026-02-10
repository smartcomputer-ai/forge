use crate::{
    AttractorError, AttractorStageToAgentLinkRecord, AttractorStorageReader, storage::types,
};
use forge_turnstore::{ContextId, StoredTurn, StoredTurnEnvelope, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const QUERY_PAGE_SIZE: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunMetadata {
    pub context_id: ContextId,
    pub run_id: Option<String>,
    pub graph_id: Option<String>,
    pub lineage_attempt: Option<u32>,
    pub started_at: Option<String>,
    pub finalized_at: Option<String>,
    pub status: Option<String>,
    pub dot_source_hash: Option<String>,
    pub dot_source_ref: Option<String>,
    pub graph_snapshot_hash: Option<String>,
    pub graph_snapshot_ref: Option<String>,
    pub head_turn_id: TurnId,
    pub head_depth: u32,
    pub turn_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageTimelineEntry {
    pub sequence_no: Option<u64>,
    pub timestamp: String,
    pub event_kind: String,
    pub node_id: Option<String>,
    pub stage_attempt_id: Option<String>,
    pub attempt: Option<u32>,
    pub status: Option<String>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub sequence_no: Option<u64>,
    pub checkpoint_id: String,
    pub timestamp: String,
    pub node_id: Option<String>,
    pub stage_attempt_id: Option<String>,
    pub checkpoint_hash: Option<String>,
    pub state_summary: Value,
}

pub async fn query_run_metadata(
    reader: &dyn AttractorStorageReader,
    context_id: &ContextId,
) -> Result<RunMetadata, AttractorError> {
    let head = reader.get_head(context_id).await?;
    let turns = collect_all_turns(reader, context_id).await?;

    let mut run_id = None;
    let mut graph_id = None;
    let mut lineage_attempt = None;
    let mut started_at = None;
    let mut finalized_at = None;
    let mut status = None;
    let mut dot_source_hash = None;
    let mut dot_source_ref = None;
    let mut graph_snapshot_hash = None;
    let mut graph_snapshot_ref = None;

    for turn in &turns {
        if turn.type_id != types::ATTRACTOR_RUN_EVENT_TYPE_ID {
            continue;
        }
        let envelope = decode_envelope(turn)?;
        if run_id.is_none() {
            run_id = envelope.run_id.clone();
        }
        match envelope.event_kind.as_str() {
            "run_initialized" => {
                started_at = Some(envelope.timestamp.clone());
                graph_id = envelope
                    .payload
                    .get("graph_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                lineage_attempt = envelope
                    .payload
                    .get("lineage_attempt")
                    .and_then(Value::as_u64)
                    .map(|value| value as u32);
                dot_source_hash = envelope
                    .payload
                    .get("dot_source_hash")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                dot_source_ref = envelope
                    .payload
                    .get("dot_source_ref")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                graph_snapshot_hash = envelope
                    .payload
                    .get("graph_snapshot_hash")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                graph_snapshot_ref = envelope
                    .payload
                    .get("graph_snapshot_ref")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            "run_finalized" => {
                finalized_at = Some(envelope.timestamp.clone());
                status = envelope
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            _ => {}
        }
    }

    Ok(RunMetadata {
        context_id: context_id.clone(),
        run_id,
        graph_id,
        lineage_attempt,
        started_at,
        finalized_at,
        status,
        dot_source_hash,
        dot_source_ref,
        graph_snapshot_hash,
        graph_snapshot_ref,
        head_turn_id: head.turn_id,
        head_depth: head.depth,
        turn_count: turns.len(),
    })
}

pub async fn query_stage_timeline(
    reader: &dyn AttractorStorageReader,
    context_id: &ContextId,
) -> Result<Vec<StageTimelineEntry>, AttractorError> {
    let turns = collect_all_turns(reader, context_id).await?;
    let mut timeline = Vec::new();
    for turn in turns {
        if turn.type_id != types::ATTRACTOR_STAGE_EVENT_TYPE_ID {
            continue;
        }
        let envelope = decode_envelope(&turn)?;
        timeline.push(StageTimelineEntry {
            sequence_no: envelope.correlation.sequence_no,
            timestamp: envelope.timestamp,
            event_kind: envelope.event_kind,
            node_id: envelope.node_id.or(envelope.correlation.node_id),
            stage_attempt_id: envelope
                .stage_attempt_id
                .or(envelope.correlation.stage_attempt_id),
            attempt: envelope
                .payload
                .get("attempt")
                .and_then(Value::as_u64)
                .map(|value| value as u32),
            status: envelope
                .payload
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            notes: envelope
                .payload
                .get("notes")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        });
    }
    Ok(timeline)
}

pub async fn query_latest_checkpoint_snapshot(
    reader: &dyn AttractorStorageReader,
    context_id: &ContextId,
) -> Result<Option<CheckpointSnapshot>, AttractorError> {
    let turns = collect_all_turns(reader, context_id).await?;
    for turn in turns.iter().rev() {
        if turn.type_id != types::ATTRACTOR_CHECKPOINT_EVENT_TYPE_ID {
            continue;
        }
        let envelope = decode_envelope(turn)?;
        let checkpoint_id = envelope
            .payload
            .get("checkpoint_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Ok(Some(CheckpointSnapshot {
            sequence_no: envelope.correlation.sequence_no,
            checkpoint_id,
            timestamp: envelope.timestamp,
            node_id: envelope.node_id.or(envelope.correlation.node_id),
            stage_attempt_id: envelope
                .stage_attempt_id
                .or(envelope.correlation.stage_attempt_id),
            checkpoint_hash: envelope
                .payload
                .get("checkpoint_hash")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            state_summary: envelope
                .payload
                .get("state_summary")
                .cloned()
                .unwrap_or(Value::Null),
        }));
    }
    Ok(None)
}

pub async fn query_stage_to_agent_linkage(
    reader: &dyn AttractorStorageReader,
    context_id: &ContextId,
) -> Result<Vec<AttractorStageToAgentLinkRecord>, AttractorError> {
    let turns = collect_all_turns(reader, context_id).await?;
    let mut links = Vec::new();
    for turn in turns {
        if turn.type_id != types::ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID {
            continue;
        }
        let envelope = decode_envelope(&turn)?;
        let payload = envelope.payload;
        let record = AttractorStageToAgentLinkRecord {
            timestamp: envelope.timestamp,
            run_id: required_string_field(&payload, "run_id")?,
            pipeline_context_id: required_string_field(&payload, "pipeline_context_id")?,
            node_id: required_string_field(&payload, "node_id")?,
            stage_attempt_id: required_string_field(&payload, "stage_attempt_id")?,
            agent_session_id: required_string_field(&payload, "agent_session_id")?,
            agent_context_id: required_string_field(&payload, "agent_context_id")?,
            agent_head_turn_id: optional_string_field(&payload, "agent_head_turn_id"),
            parent_turn_id: optional_string_field(&payload, "parent_turn_id"),
            sequence_no: payload
                .get("sequence_no")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            thread_key: optional_string_field(&payload, "thread_key"),
        };
        links.push(record);
    }
    Ok(links)
}

async fn collect_all_turns(
    reader: &dyn AttractorStorageReader,
    context_id: &ContextId,
) -> Result<Vec<StoredTurn>, AttractorError> {
    let mut before_turn_id: Option<TurnId> = None;
    let mut pages: Vec<Vec<StoredTurn>> = Vec::new();

    loop {
        let page = reader
            .list_turns(context_id, before_turn_id.as_ref(), QUERY_PAGE_SIZE)
            .await?;
        if page.is_empty() {
            break;
        }
        before_turn_id = page.first().map(|turn| turn.turn_id.clone());
        let should_continue = page.len() == QUERY_PAGE_SIZE;
        pages.push(page);
        if !should_continue {
            break;
        }
    }

    let mut turns = Vec::new();
    for page in pages.into_iter().rev() {
        turns.extend(page);
    }
    Ok(turns)
}

fn decode_envelope(turn: &StoredTurn) -> Result<StoredTurnEnvelope, AttractorError> {
    serde_json::from_slice::<StoredTurnEnvelope>(&turn.payload).map_err(|error| {
        AttractorError::Runtime(format!(
            "failed to decode stored turn envelope for type '{}': {error}",
            turn.type_id
        ))
    })
}

fn required_string_field(payload: &Value, key: &str) -> Result<String, AttractorError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            AttractorError::Runtime(format!(
                "stage-to-agent linkage payload is missing required field '{key}'"
            ))
        })
}

fn optional_string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
