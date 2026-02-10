use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;
use std::sync::Arc;

pub type ContextId = String;
pub type TurnId = String;
pub type BlobHash = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreContext {
    pub context_id: ContextId,
    pub head_turn_id: TurnId,
    pub head_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredTurnRef {
    pub context_id: ContextId,
    pub turn_id: TurnId,
    pub depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendTurnRequest {
    pub context_id: ContextId,
    pub parent_turn_id: Option<TurnId>,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryBundle {
    pub bundle_id: String,
    pub bundle_json: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum TurnStoreError {
    #[error("resource not found: {resource} ({id})")]
    NotFound { resource: &'static str, id: String },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("backend failure: {0}")]
    Backend(String),
}

pub type TurnStoreResult<T> = Result<T, TurnStoreError>;

#[async_trait]
pub trait TurnStore: Send + Sync {
    async fn create_context(&self, base_turn_id: Option<TurnId>) -> TurnStoreResult<StoreContext>;
    async fn append_turn(&self, request: AppendTurnRequest) -> TurnStoreResult<StoredTurn>;
    async fn fork_context(&self, from_turn_id: TurnId) -> TurnStoreResult<StoreContext>;
    async fn get_head(&self, context_id: &ContextId) -> TurnStoreResult<StoredTurnRef>;
    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> TurnStoreResult<Vec<StoredTurn>>;
}

#[async_trait]
pub trait TypedTurnStore: TurnStore {
    async fn publish_registry_bundle(&self, bundle: RegistryBundle) -> TurnStoreResult<()>;
    async fn get_registry_bundle(&self, bundle_id: &str) -> TurnStoreResult<Option<Vec<u8>>>;
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put_blob(&self, raw_bytes: &[u8]) -> TurnStoreResult<BlobHash>;
    async fn get_blob(&self, content_hash: &BlobHash) -> TurnStoreResult<Option<Vec<u8>>>;
    async fn attach_fs(&self, turn_id: &TurnId, fs_root_hash: &BlobHash) -> TurnStoreResult<()>;
}

pub const DEFAULT_CXDB_BINARY_ADDR: &str = "127.0.0.1:9009";
pub const DEFAULT_CXDB_HTTP_BASE_URL: &str = "http://127.0.0.1:9010";

#[derive(Debug, thiserror::Error)]
pub enum CxdbClientError {
    #[error("resource not found: {resource} ({id})")]
    NotFound { resource: &'static str, id: String },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("backend failure: {0}")]
    Backend(String),
}

impl CxdbClientError {
    fn into_turnstore_error(self) -> TurnStoreError {
        match self {
            Self::NotFound { resource, id } => TurnStoreError::NotFound { resource, id },
            Self::Conflict(message) => TurnStoreError::Conflict(message),
            Self::InvalidInput(message) => TurnStoreError::InvalidInput(message),
            Self::Backend(message) => TurnStoreError::Backend(message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryContextHead {
    pub context_id: u64,
    pub head_turn_id: u64,
    pub head_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryAppendTurnRequest {
    pub context_id: u64,
    pub parent_turn_id: u64,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: String,
    pub content_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryAppendTurnResponse {
    pub context_id: u64,
    pub new_turn_id: u64,
    pub new_depth: u32,
    pub content_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryStoredTurn {
    pub context_id: u64,
    pub turn_id: u64,
    pub parent_turn_id: u64,
    pub depth: u32,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: Option<String>,
    pub content_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpStoredTurn {
    pub context_id: u64,
    pub turn_id: u64,
    pub parent_turn_id: u64,
    pub depth: u32,
    pub type_id: String,
    pub type_version: u32,
    pub payload: Vec<u8>,
    pub idempotency_key: Option<String>,
    pub content_hash: [u8; 32],
}

#[async_trait]
pub trait CxdbBinaryClient: Send + Sync {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError>;
    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError>;
    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError>;
    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError>;
    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError>;
    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, CxdbClientError>;
    async fn get_blob(&self, content_hash: &BlobHash) -> Result<Option<Vec<u8>>, CxdbClientError>;
    async fn attach_fs(&self, turn_id: u64, fs_root_hash: &BlobHash)
    -> Result<(), CxdbClientError>;
}

#[async_trait]
impl<T> CxdbBinaryClient for std::sync::Arc<T>
where
    T: CxdbBinaryClient + ?Sized,
{
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        (**self).ctx_create(base_turn_id).await
    }

    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        (**self).ctx_fork(from_turn_id).await
    }

    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        (**self).append_turn(request).await
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        (**self).get_head(context_id).await
    }

    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        (**self).get_last(context_id, limit, include_payload).await
    }

    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, CxdbClientError> {
        (**self).put_blob(raw_bytes).await
    }

    async fn get_blob(&self, content_hash: &BlobHash) -> Result<Option<Vec<u8>>, CxdbClientError> {
        (**self).get_blob(content_hash).await
    }

    async fn attach_fs(
        &self,
        turn_id: u64,
        fs_root_hash: &BlobHash,
    ) -> Result<(), CxdbClientError> {
        (**self).attach_fs(turn_id, fs_root_hash).await
    }
}

#[async_trait]
pub trait CxdbHttpClient: Send + Sync {
    async fn list_turns(
        &self,
        context_id: u64,
        before_turn_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError>;

    async fn publish_registry_bundle(
        &self,
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError>;

    async fn get_registry_bundle(
        &self,
        bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError>;
}

#[async_trait]
impl<T> CxdbHttpClient for std::sync::Arc<T>
where
    T: CxdbHttpClient + ?Sized,
{
    async fn list_turns(
        &self,
        context_id: u64,
        before_turn_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        (**self).list_turns(context_id, before_turn_id, limit).await
    }

    async fn publish_registry_bundle(
        &self,
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        (**self)
            .publish_registry_bundle(bundle_id, bundle_json)
            .await
    }

    async fn get_registry_bundle(
        &self,
        bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        (**self).get_registry_bundle(bundle_id).await
    }
}

#[derive(Clone)]
pub struct CxdbSdkBinaryClient {
    client: Arc<cxdb::Client>,
}

impl CxdbSdkBinaryClient {
    pub fn connect(binary_addr: &str) -> Result<Self, CxdbClientError> {
        let client = cxdb::dial(binary_addr, Vec::new()).map_err(map_cxdb_error)?;
        Ok(Self {
            client: Arc::new(client),
        })
    }

    pub fn from_client(client: cxdb::Client) -> Self {
        Self {
            client: Arc::new(client),
        }
    }

    pub fn from_shared_client(client: Arc<cxdb::Client>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl CxdbBinaryClient for CxdbSdkBinaryClient {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let head = self
            .client
            .create_context(&request_context, base_turn_id)
            .map_err(map_cxdb_error)?;
        Ok(BinaryContextHead {
            context_id: head.context_id,
            head_turn_id: head.head_turn_id,
            head_depth: head.head_depth,
        })
    }

    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let head = self
            .client
            .fork_context(&request_context, from_turn_id)
            .map_err(map_cxdb_error)?;
        Ok(BinaryContextHead {
            context_id: head.context_id,
            head_turn_id: head.head_turn_id,
            head_depth: head.head_depth,
        })
    }

    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let req = cxdb::AppendRequest {
            context_id: request.context_id,
            parent_turn_id: request.parent_turn_id,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: request.idempotency_key.into_bytes(),
            encoding: cxdb::EncodingMsgpack,
            compression: cxdb::CompressionNone,
        };
        let appended = self
            .client
            .append_turn(&request_context, &req)
            .map_err(map_cxdb_error)?;
        Ok(BinaryAppendTurnResponse {
            context_id: appended.context_id,
            new_turn_id: appended.turn_id,
            new_depth: appended.depth,
            content_hash: appended.payload_hash,
        })
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let head = self
            .client
            .get_head(&request_context, context_id)
            .map_err(map_cxdb_error)?;
        Ok(BinaryContextHead {
            context_id: head.context_id,
            head_turn_id: head.head_turn_id,
            head_depth: head.head_depth,
        })
    }

    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let turns = self
            .client
            .get_last(
                &request_context,
                context_id,
                cxdb::GetLastOptions {
                    limit: limit.min(u32::MAX as usize) as u32,
                    include_payload,
                },
            )
            .map_err(map_cxdb_error)?;

        Ok(turns
            .into_iter()
            .map(|turn| BinaryStoredTurn {
                context_id,
                turn_id: turn.turn_id,
                parent_turn_id: turn.parent_id,
                depth: turn.depth,
                type_id: turn.type_id,
                type_version: turn.type_version,
                payload: turn.payload,
                idempotency_key: None,
                content_hash: turn.payload_hash,
            })
            .collect())
    }

    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<BlobHash, CxdbClientError> {
        let request_context = cxdb::RequestContext::background();
        let result = self
            .client
            .put_blob(
                &request_context,
                &cxdb::PutBlobRequest {
                    data: raw_bytes.to_vec(),
                },
            )
            .map_err(map_cxdb_error)?;
        Ok(hash_hex(result.hash))
    }

    async fn get_blob(&self, _content_hash: &BlobHash) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "GET_BLOB is not exposed by the vendored cxdb client yet".to_string(),
        ))
    }

    async fn attach_fs(
        &self,
        turn_id: u64,
        fs_root_hash: &BlobHash,
    ) -> Result<(), CxdbClientError> {
        let parsed_hash = parse_hex_32(fs_root_hash).ok_or_else(|| {
            CxdbClientError::InvalidInput(format!(
                "fs_root_hash must be a 64-character lowercase hex BLAKE3 digest: {fs_root_hash}"
            ))
        })?;
        let request_context = cxdb::RequestContext::background();
        self.client
            .attach_fs(
                &request_context,
                &cxdb::AttachFsRequest {
                    turn_id,
                    fs_root_hash: parsed_hash,
                },
            )
            .map_err(map_cxdb_error)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CxdbReqwestHttpClient {
    client: reqwest::Client,
    base_url: String,
}

impl CxdbReqwestHttpClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("CXDB_HTTP_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CXDB_HTTP_BASE_URL.to_string());
        Self::new(base_url)
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

#[async_trait]
impl CxdbHttpClient for CxdbReqwestHttpClient {
    async fn list_turns(
        &self,
        context_id: u64,
        before_turn_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        let mut path = format!("/v1/contexts/{context_id}/turns?limit={limit}&view=both");
        if let Some(before) = before_turn_id {
            path.push_str(&format!("&before_turn_id={before}"));
        }

        let response = self
            .client
            .get(self.endpoint(&path))
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http get failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http read body failed: {err}")))?;
        if !status.is_success() {
            return Err(map_http_status(status, text));
        }
        let payload: Value = serde_json::from_str(&text)
            .map_err(|err| CxdbClientError::Backend(format!("http json decode failed: {err}")))?;
        let turns = payload
            .get("turns")
            .and_then(Value::as_array)
            .ok_or_else(|| CxdbClientError::Backend("missing turns array".to_string()))?;

        turns
            .iter()
            .map(|turn| parse_http_turn(turn, context_id))
            .collect()
    }

    async fn publish_registry_bundle(
        &self,
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        let response = self
            .client
            .put(self.endpoint(&format!("/v1/registry/bundles/{bundle_id}")))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bundle_json.to_vec())
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http put failed: {err}")))?;

        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let text = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable body>".to_string());
        Err(map_http_status(status, text))
    }

    async fn get_registry_bundle(
        &self,
        bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        let response = self
            .client
            .get(self.endpoint(&format!("/v1/registry/bundles/{bundle_id}")))
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http get failed: {err}")))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(map_http_status(status, text));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http read body failed: {err}")))?;
        Ok(Some(bytes.to_vec()))
    }
}

#[derive(Clone, Debug)]
pub struct CxdbTurnStore<B, H> {
    binary_client: B,
    http_client: H,
}

impl<B, H> CxdbTurnStore<B, H> {
    pub fn new(binary_client: B, http_client: H) -> Self {
        Self {
            binary_client,
            http_client,
        }
    }
}

impl CxdbTurnStore<CxdbSdkBinaryClient, CxdbReqwestHttpClient> {
    pub fn connect(binary_addr: &str, http_base_url: &str) -> Result<Self, CxdbClientError> {
        Ok(Self::new(
            CxdbSdkBinaryClient::connect(binary_addr)?,
            CxdbReqwestHttpClient::new(http_base_url),
        ))
    }

    pub fn connect_default() -> Result<Self, CxdbClientError> {
        Self::connect(DEFAULT_CXDB_BINARY_ADDR, DEFAULT_CXDB_HTTP_BASE_URL)
    }

    pub fn connect_from_env() -> Result<Self, CxdbClientError> {
        let binary_addr = std::env::var("CXDB_ADDR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("CXDB_BINARY_ADDR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| DEFAULT_CXDB_BINARY_ADDR.to_string());

        let http_base_url = std::env::var("CXDB_HTTP_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CXDB_HTTP_BASE_URL.to_string());

        Self::connect(&binary_addr, &http_base_url)
    }
}

impl<B, H> CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient,
    H: CxdbHttpClient,
{
    fn parse_context_id(context_id: &ContextId) -> TurnStoreResult<u64> {
        context_id.parse::<u64>().map_err(|_| {
            TurnStoreError::InvalidInput(format!(
                "context_id must be a u64-compatible string: {context_id}"
            ))
        })
    }

    fn parse_turn_id(turn_id: &TurnId) -> TurnStoreResult<u64> {
        turn_id.parse::<u64>().map_err(|_| {
            TurnStoreError::InvalidInput(format!(
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

    fn as_stored_turn(turn: BinaryStoredTurn) -> StoredTurn {
        StoredTurn {
            context_id: Self::context_id_string(turn.context_id),
            turn_id: Self::turn_id_string(turn.turn_id),
            parent_turn_id: Self::turn_id_string(turn.parent_turn_id),
            depth: turn.depth,
            type_id: turn.type_id,
            type_version: turn.type_version,
            payload: turn.payload,
            idempotency_key: turn.idempotency_key,
            content_hash: Some(Self::hash_hex(turn.content_hash)),
        }
    }

    fn as_stored_turn_from_http(turn: HttpStoredTurn) -> StoredTurn {
        StoredTurn {
            context_id: Self::context_id_string(turn.context_id),
            turn_id: Self::turn_id_string(turn.turn_id),
            parent_turn_id: Self::turn_id_string(turn.parent_turn_id),
            depth: turn.depth,
            type_id: turn.type_id,
            type_version: turn.type_version,
            payload: turn.payload,
            idempotency_key: turn.idempotency_key,
            content_hash: Some(Self::hash_hex(turn.content_hash)),
        }
    }
}

#[async_trait]
impl<B, H> TurnStore for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient,
    H: CxdbHttpClient,
{
    async fn create_context(&self, base_turn_id: Option<TurnId>) -> TurnStoreResult<StoreContext> {
        let base_turn_id = match base_turn_id {
            Some(turn_id) if turn_id != "0" => Self::parse_turn_id(&turn_id)?,
            _ => 0,
        };

        let created = self
            .binary_client
            .ctx_create(base_turn_id)
            .await
            .map_err(CxdbClientError::into_turnstore_error)?;

        Ok(StoreContext {
            context_id: Self::context_id_string(created.context_id),
            head_turn_id: Self::turn_id_string(created.head_turn_id),
            head_depth: created.head_depth,
        })
    }

    async fn append_turn(&self, request: AppendTurnRequest) -> TurnStoreResult<StoredTurn> {
        let context_id = Self::parse_context_id(&request.context_id)?;

        let parent_turn_id = match request.parent_turn_id.as_ref() {
            Some(turn_id) if turn_id != "0" => Self::parse_turn_id(turn_id)?,
            _ => 0,
        };

        let resolved_parent_turn_id = if parent_turn_id == 0 {
            self.binary_client
                .get_head(context_id)
                .await
                .map_err(CxdbClientError::into_turnstore_error)?
                .head_turn_id
        } else {
            parent_turn_id
        };

        let content_hash = *blake3::hash(&request.payload).as_bytes();
        let content_hash_hex = Self::hash_hex(content_hash);
        let idempotency_key = if request.idempotency_key.is_empty() {
            Self::deterministic_idempotency_key(
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
                parent_turn_id,
                type_id: request_type_id.clone(),
                type_version: request_type_version,
                payload: request_payload.clone(),
                idempotency_key: idempotency_key.clone(),
                content_hash,
            })
            .await
            .map_err(CxdbClientError::into_turnstore_error)?;

        let committed_parent_turn_id = if parent_turn_id == 0 {
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
            parent_turn_id
        };

        Ok(StoredTurn {
            context_id: Self::context_id_string(appended.context_id),
            turn_id: Self::turn_id_string(appended.new_turn_id),
            parent_turn_id: Self::turn_id_string(committed_parent_turn_id),
            depth: appended.new_depth,
            type_id: request_type_id,
            type_version: request_type_version,
            payload: request_payload,
            idempotency_key: Some(idempotency_key),
            content_hash: Some(Self::hash_hex(appended.content_hash)),
        })
    }

    async fn fork_context(&self, from_turn_id: TurnId) -> TurnStoreResult<StoreContext> {
        let from_turn_id = Self::parse_turn_id(&from_turn_id)?;
        let forked = self
            .binary_client
            .ctx_fork(from_turn_id)
            .await
            .map_err(CxdbClientError::into_turnstore_error)?;

        Ok(StoreContext {
            context_id: Self::context_id_string(forked.context_id),
            head_turn_id: Self::turn_id_string(forked.head_turn_id),
            head_depth: forked.head_depth,
        })
    }

    async fn get_head(&self, context_id: &ContextId) -> TurnStoreResult<StoredTurnRef> {
        let context_id_u64 = Self::parse_context_id(context_id)?;
        let head = self
            .binary_client
            .get_head(context_id_u64)
            .await
            .map_err(CxdbClientError::into_turnstore_error)?;

        Ok(StoredTurnRef {
            context_id: Self::context_id_string(head.context_id),
            turn_id: Self::turn_id_string(head.head_turn_id),
            depth: head.head_depth,
        })
    }

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> TurnStoreResult<Vec<StoredTurn>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let context_id_u64 = Self::parse_context_id(context_id)?;

        if let Some(turn_id) = before_turn_id {
            if turn_id == "0" {
                return Ok(Vec::new());
            }
            let before_turn_id_u64 = Self::parse_turn_id(turn_id)?;
            let turns = self
                .http_client
                .list_turns(context_id_u64, Some(before_turn_id_u64), limit)
                .await
                .map_err(CxdbClientError::into_turnstore_error)?;
            return Ok(turns
                .into_iter()
                .map(Self::as_stored_turn_from_http)
                .collect());
        }

        let turns = self
            .binary_client
            .get_last(context_id_u64, limit, true)
            .await
            .map_err(CxdbClientError::into_turnstore_error)?;

        Ok(turns.into_iter().map(Self::as_stored_turn).collect())
    }
}

#[async_trait]
impl<B, H> TypedTurnStore for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient,
    H: CxdbHttpClient,
{
    async fn publish_registry_bundle(&self, bundle: RegistryBundle) -> TurnStoreResult<()> {
        self.http_client
            .publish_registry_bundle(&bundle.bundle_id, &bundle.bundle_json)
            .await
            .map_err(CxdbClientError::into_turnstore_error)
    }

    async fn get_registry_bundle(&self, bundle_id: &str) -> TurnStoreResult<Option<Vec<u8>>> {
        self.http_client
            .get_registry_bundle(bundle_id)
            .await
            .map_err(CxdbClientError::into_turnstore_error)
    }
}

#[async_trait]
impl<B, H> ArtifactStore for CxdbTurnStore<B, H>
where
    B: CxdbBinaryClient,
    H: CxdbHttpClient,
{
    async fn put_blob(&self, raw_bytes: &[u8]) -> TurnStoreResult<BlobHash> {
        self.binary_client
            .put_blob(raw_bytes)
            .await
            .map_err(CxdbClientError::into_turnstore_error)
    }

    async fn get_blob(&self, content_hash: &BlobHash) -> TurnStoreResult<Option<Vec<u8>>> {
        self.binary_client
            .get_blob(content_hash)
            .await
            .map_err(CxdbClientError::into_turnstore_error)
    }

    async fn attach_fs(&self, turn_id: &TurnId, fs_root_hash: &BlobHash) -> TurnStoreResult<()> {
        let turn_id_u64 = Self::parse_turn_id(turn_id)?;
        self.binary_client
            .attach_fs(turn_id_u64, fs_root_hash)
            .await
            .map_err(CxdbClientError::into_turnstore_error)
    }
}

fn map_cxdb_error(error: cxdb::Error) -> CxdbClientError {
    match error {
        cxdb::Error::ContextNotFound => CxdbClientError::NotFound {
            resource: "context",
            id: "unknown".to_string(),
        },
        cxdb::Error::TurnNotFound => CxdbClientError::NotFound {
            resource: "turn",
            id: "unknown".to_string(),
        },
        cxdb::Error::InvalidResponse(message) => {
            CxdbClientError::Backend(format!("cxdb invalid response: {message}"))
        }
        cxdb::Error::Server(server_error) => match server_error.code {
            404 => CxdbClientError::NotFound {
                resource: "resource",
                id: server_error.detail,
            },
            409 => CxdbClientError::Conflict(server_error.detail),
            422 => CxdbClientError::InvalidInput(server_error.detail),
            _ => CxdbClientError::Backend(format!(
                "cxdb server error {}: {}",
                server_error.code, server_error.detail
            )),
        },
        other => CxdbClientError::Backend(other.to_string()),
    }
}

fn map_http_status(status: reqwest::StatusCode, body: String) -> CxdbClientError {
    match status {
        reqwest::StatusCode::NOT_FOUND => CxdbClientError::NotFound {
            resource: "resource",
            id: body,
        },
        reqwest::StatusCode::CONFLICT => CxdbClientError::Conflict(body),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY | reqwest::StatusCode::BAD_REQUEST => {
            CxdbClientError::InvalidInput(body)
        }
        _ => CxdbClientError::Backend(format!("http request failed with status {status}: {body}")),
    }
}

fn parse_http_turn(turn: &Value, context_id: u64) -> Result<HttpStoredTurn, CxdbClientError> {
    let turn_id = parse_u64_field(turn, "turn_id")?;
    let parent_turn_id = parse_u64_field(turn, "parent_turn_id")?;
    let depth = parse_u32_field(turn, "depth")?;
    let declared_type = turn
        .get("declared_type")
        .ok_or_else(|| CxdbClientError::Backend("missing declared_type".to_string()))?;
    let type_id = declared_type
        .get("type_id")
        .and_then(Value::as_str)
        .ok_or_else(|| CxdbClientError::Backend("missing declared_type.type_id".to_string()))?
        .to_string();
    let type_version = declared_type
        .get("type_version")
        .and_then(Value::as_u64)
        .map(|value| value as u32)
        .ok_or_else(|| {
            CxdbClientError::Backend("missing declared_type.type_version".to_string())
        })?;
    let payload = decode_turn_payload(turn)?;
    let content_hash = parse_hash_hex(turn, "content_hash_b3")
        .or_else(|| parse_hash_hex(turn, "content_hash"))
        .unwrap_or(*blake3::hash(&payload).as_bytes());

    Ok(HttpStoredTurn {
        context_id,
        turn_id,
        parent_turn_id,
        depth,
        type_id,
        type_version,
        payload,
        idempotency_key: None,
        content_hash,
    })
}

fn parse_u64_field(payload: &Value, key: &str) -> Result<u64, CxdbClientError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| payload.get(key).and_then(Value::as_u64))
        .ok_or_else(|| CxdbClientError::Backend(format!("missing or invalid field '{key}'")))
}

fn parse_u32_field(payload: &Value, key: &str) -> Result<u32, CxdbClientError> {
    payload
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as u32)
        .ok_or_else(|| CxdbClientError::Backend(format!("missing or invalid field '{key}'")))
}

fn decode_turn_payload(turn: &Value) -> Result<Vec<u8>, CxdbClientError> {
    if let Some(bytes_b64) = turn.get("bytes_b64").and_then(Value::as_str) {
        return base64::engine::general_purpose::STANDARD
            .decode(bytes_b64)
            .map_err(|err| CxdbClientError::Backend(format!("bytes_b64 decode failed: {err}")));
    }
    if let Some(data) = turn.get("data") {
        if let Some(payload_b64) = data.get("payload_b64").and_then(Value::as_str) {
            return base64::engine::general_purpose::STANDARD
                .decode(payload_b64)
                .map_err(|err| {
                    CxdbClientError::Backend(format!("payload_b64 decode failed: {err}"))
                });
        }
        return serde_json::to_vec(data)
            .map_err(|err| CxdbClientError::Backend(format!("data encode failed: {err}")));
    }
    Err(CxdbClientError::Backend(
        "turn payload has neither bytes_b64 nor data".to_string(),
    ))
}

fn parse_hash_hex(payload: &Value, key: &str) -> Option<[u8; 32]> {
    let raw = payload.get(key).and_then(Value::as_str)?;
    parse_hex_32(raw)
}

fn parse_hex_32(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in input.as_bytes().chunks_exact(2).enumerate() {
        let high = (chunk[0] as char).to_digit(16)?;
        let low = (chunk[1] as char).to_digit(16)?;
        out[index] = ((high << 4) | low) as u8;
    }
    Some(out)
}

fn hash_hex(hash: [u8; 32]) -> BlobHash {
    let mut hex = String::with_capacity(64);
    for byte in hash {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

fn encode_part(part: &str) -> String {
    format!("{}:{}", part.len(), part)
}
