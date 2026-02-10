use crate::{
    BinaryAppendTurnRequest, CxdbBinaryClient, CxdbClientError, CxdbHttpClient, HttpStoredTurn,
};
use serde::{Deserialize, Serialize};

pub type ContextId = String;
pub type TurnId = String;
pub type BlobHash = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreContext {
    pub context_id: ContextId,
    pub head_turn_id: TurnId,
    pub head_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTurnRef {
    pub context_id: ContextId,
    pub turn_id: TurnId,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendTurnRequest {
    pub context_id: ContextId,
    pub parent_turn_id: Option<TurnId>,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTurn {
    pub context_id: ContextId,
    pub turn_id: TurnId,
    pub parent_turn_id: TurnId,
    pub depth: u32,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: Option<String>,
    pub content_hash: Option<BlobHash>,
}

#[derive(Clone, Debug)]
pub struct CxdbRuntimeStore<B, H> {
    binary_client: B,
    http_client: H,
}

impl<B, H> CxdbRuntimeStore<B, H> {
    pub fn new(binary_client: B, http_client: H) -> Self {
        Self {
            binary_client,
            http_client,
        }
    }

    pub fn binary_client(&self) -> &B {
        &self.binary_client
    }

    pub fn http_client(&self) -> &H {
        &self.http_client
    }
}

impl<B, H> CxdbRuntimeStore<B, H>
where
    B: CxdbBinaryClient,
    H: CxdbHttpClient,
{
    pub async fn create_context(
        &self,
        base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, CxdbClientError> {
        let base_turn_id = match base_turn_id {
            Some(turn_id) if turn_id != "0" => parse_turn_id(&turn_id)?,
            _ => 0,
        };

        let created = self.binary_client.ctx_create(base_turn_id).await?;
        Ok(StoreContext {
            context_id: context_id_string(created.context_id),
            head_turn_id: turn_id_string(created.head_turn_id),
            head_depth: created.head_depth,
        })
    }

    pub async fn fork_context(
        &self,
        from_turn_id: TurnId,
    ) -> Result<StoreContext, CxdbClientError> {
        let from_turn_id = parse_turn_id(&from_turn_id)?;
        let forked = self.binary_client.ctx_fork(from_turn_id).await?;

        Ok(StoreContext {
            context_id: context_id_string(forked.context_id),
            head_turn_id: turn_id_string(forked.head_turn_id),
            head_depth: forked.head_depth,
        })
    }

    pub async fn append_turn(
        &self,
        request: AppendTurnRequest,
    ) -> Result<StoredTurn, CxdbClientError> {
        let context_id = parse_context_id(&request.context_id)?;

        let requested_parent_turn_id = match request.parent_turn_id.as_ref() {
            Some(turn_id) if turn_id != "0" => parse_turn_id(turn_id)?,
            _ => 0,
        };

        let resolved_parent_turn_id = if requested_parent_turn_id == 0 {
            self.binary_client.get_head(context_id).await?.head_turn_id
        } else {
            requested_parent_turn_id
        };

        let content_hash = *blake3::hash(&request.payload).as_bytes();
        let content_hash_hex = hash_hex(content_hash);
        let idempotency_key = if request.idempotency_key.is_empty() {
            deterministic_idempotency_key(
                context_id,
                resolved_parent_turn_id,
                &request.type_id,
                request.type_version,
                &content_hash_hex,
            )
        } else {
            request.idempotency_key.clone()
        };

        let request_payload = request.payload;
        let request_type_id = request.type_id;
        let request_type_version = request.type_version;

        let appended = self
            .binary_client
            .append_turn(BinaryAppendTurnRequest {
                context_id,
                parent_turn_id: requested_parent_turn_id,
                type_id: request_type_id.clone(),
                type_version: request_type_version,
                payload: request_payload.clone(),
                idempotency_key: idempotency_key.clone(),
                content_hash,
            })
            .await?;

        let committed_parent_turn_id = if requested_parent_turn_id == 0 {
            self.binary_client
                .get_last(context_id, 1, false)
                .await
                .ok()
                .and_then(|turns| {
                    turns
                        .into_iter()
                        .find(|turn| turn.turn_id == appended.new_turn_id)
                })
                .map(|turn| turn.parent_turn_id)
                .unwrap_or(resolved_parent_turn_id)
        } else {
            requested_parent_turn_id
        };

        Ok(StoredTurn {
            context_id: context_id_string(appended.context_id),
            turn_id: turn_id_string(appended.new_turn_id),
            parent_turn_id: turn_id_string(committed_parent_turn_id),
            depth: appended.new_depth,
            type_id: request_type_id,
            type_version: request_type_version,
            payload: request_payload,
            idempotency_key: Some(idempotency_key),
            content_hash: Some(hash_hex(appended.content_hash)),
        })
    }

    pub async fn get_head(&self, context_id: &ContextId) -> Result<StoredTurnRef, CxdbClientError> {
        let context_id_u64 = parse_context_id(context_id)?;
        let head = self.binary_client.get_head(context_id_u64).await?;

        Ok(StoredTurnRef {
            context_id: context_id_string(head.context_id),
            turn_id: turn_id_string(head.head_turn_id),
            depth: head.head_depth,
        })
    }

    pub async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> Result<Vec<StoredTurn>, CxdbClientError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let context_id_u64 = parse_context_id(context_id)?;
        let before_turn_id_u64 = match before_turn_id {
            Some(turn_id) if turn_id == "0" => return Ok(Vec::new()),
            Some(turn_id) => Some(parse_turn_id(turn_id)?),
            None => None,
        };
        let turns = self
            .http_client
            .list_turns(context_id_u64, before_turn_id_u64, limit)
            .await?;
        Ok(turns.into_iter().map(stored_turn_from_http).collect())
    }

    pub async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, CxdbClientError> {
        self.binary_client.put_blob(raw_bytes).await
    }

    pub async fn get_blob(
        &self,
        content_hash: &BlobHash,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        self.binary_client.get_blob(content_hash).await
    }

    pub async fn attach_fs(
        &self,
        turn_id: &TurnId,
        fs_root_hash: &BlobHash,
    ) -> Result<(), CxdbClientError> {
        let turn_id_u64 = parse_turn_id(turn_id)?;
        self.binary_client
            .attach_fs(turn_id_u64, fs_root_hash)
            .await
    }

    pub async fn publish_registry_bundle(
        &self,
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        self.http_client
            .publish_registry_bundle(bundle_id, bundle_json)
            .await
    }

    pub async fn get_registry_bundle(
        &self,
        bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        self.http_client.get_registry_bundle(bundle_id).await
    }
}

fn parse_context_id(context_id: &ContextId) -> Result<u64, CxdbClientError> {
    context_id.parse::<u64>().map_err(|_| {
        CxdbClientError::InvalidInput(format!(
            "context_id must be a u64-compatible string: {context_id}"
        ))
    })
}

fn parse_turn_id(turn_id: &TurnId) -> Result<u64, CxdbClientError> {
    turn_id.parse::<u64>().map_err(|_| {
        CxdbClientError::InvalidInput(format!(
            "turn_id must be a u64-compatible string: {turn_id}"
        ))
    })
}

fn turn_id_string(turn_id: u64) -> TurnId {
    turn_id.to_string()
}

fn context_id_string(context_id: u64) -> ContextId {
    context_id.to_string()
}

fn stored_turn_from_http(turn: HttpStoredTurn) -> StoredTurn {
    StoredTurn {
        context_id: context_id_string(turn.context_id),
        turn_id: turn_id_string(turn.turn_id),
        parent_turn_id: turn_id_string(turn.parent_turn_id),
        depth: turn.depth,
        type_id: turn.type_id,
        type_version: turn.type_version,
        payload: turn.payload,
        idempotency_key: turn.idempotency_key,
        content_hash: Some(hash_hex(turn.content_hash)),
    }
}

fn hash_hex(hash: [u8; 32]) -> BlobHash {
    let mut hex = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

fn deterministic_idempotency_key(
    context_id: u64,
    parent_turn_id: u64,
    type_id: &str,
    type_version: u32,
    content_hash_hex: &str,
) -> String {
    format!(
        "forge-cxdb:v1|ctx={context_id}|parent={parent_turn_id}|type={}:{}|hash={content_hash_hex}",
        encode_part(type_id),
        type_version
    )
}

fn encode_part(part: &str) -> String {
    format!("{}:{}", part.len(), part)
}
