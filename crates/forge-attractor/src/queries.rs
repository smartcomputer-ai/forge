use crate::storage::{
    ContextId, StoredTurn, TurnId, decode_typed_record,
};
use crate::{
    AttractorCheckpointSavedRecord, AttractorError, AttractorRunLifecycleRecord,
    AttractorStageLifecycleRecord, AttractorStageToAgentLinkRecord, AttractorStorageReader,
    storage::types,
};
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
        if turn.type_id != types::ATTRACTOR_RUN_LIFECYCLE_TYPE_ID {
            continue;
        }
        let record: AttractorRunLifecycleRecord = decode_record(turn)?;
        if run_id.is_none() {
            run_id = Some(record.run_id.clone());
        }
        match record.kind.as_str() {
            "initialized" => {
                started_at = Some(record.timestamp.clone());
                graph_id = Some(record.graph_id.clone());
                lineage_attempt = Some(record.lineage_attempt);
                dot_source_hash = record.dot_source_hash.clone();
                dot_source_ref = record.dot_source_ref.clone();
                graph_snapshot_hash = record.graph_snapshot_hash.clone();
                graph_snapshot_ref = record.graph_snapshot_ref.clone();
            }
            "finalized" => {
                finalized_at = Some(record.timestamp.clone());
                status = record.status.clone();
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
        if turn.type_id != types::ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID {
            continue;
        }
        let record: AttractorStageLifecycleRecord = decode_record(&turn)?;
        timeline.push(StageTimelineEntry {
            sequence_no: Some(record.sequence_no),
            timestamp: record.timestamp,
            event_kind: record.kind,
            node_id: Some(record.node_id),
            stage_attempt_id: Some(record.stage_attempt_id),
            attempt: Some(record.attempt),
            status: record.status,
            notes: record.notes,
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
        if turn.type_id != types::ATTRACTOR_CHECKPOINT_SAVED_TYPE_ID {
            continue;
        }
        let record: AttractorCheckpointSavedRecord = decode_record(turn)?;
        return Ok(Some(CheckpointSnapshot {
            sequence_no: Some(record.sequence_no),
            checkpoint_id: record.checkpoint_id,
            timestamp: record.timestamp,
            node_id: Some(record.node_id),
            stage_attempt_id: Some(record.stage_attempt_id),
            checkpoint_hash: record.checkpoint_hash,
            state_summary: record.state_summary,
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
        let record: AttractorStageToAgentLinkRecord = decode_record(&turn)?;
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

fn decode_record<T: serde::de::DeserializeOwned>(turn: &StoredTurn) -> Result<T, AttractorError> {
    decode_typed_record(&turn.payload).map_err(|error| {
        AttractorError::Runtime(format!(
            "failed to decode typed record for type '{}': {error}",
            turn.type_id
        ))
    })
}
