use async_trait::async_trait;
use base64::Engine;
use forge_cxdb_runtime::adapter::{
    AppendTurnRequest, CxdbRecordStore, CxdbRegistryStore, RegistryBundle,
};
use forge_cxdb_runtime::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbSdkBinaryClient, CxdbStoreAdapter,
    HttpStoredTurn,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CXDB_BINARY_ADDR: &str = "127.0.0.1:9009";
const DEFAULT_CXDB_HTTP_BASE_URL: &str = "http://127.0.0.1:9010";

#[derive(Clone)]
struct LiveHttpClient {
    client: reqwest::Client,
    base_url: String,
    supports_context_write_routes: bool,
}

impl LiveHttpClient {
    fn from_env() -> Self {
        let base_url = std::env::var("FORGE_CXDB_HTTP_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("CXDB_HTTP_BASE_URL")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| DEFAULT_CXDB_HTTP_BASE_URL.to_string());
        Self {
            client: reqwest::Client::new(),
            base_url,
            supports_context_write_routes: false,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    async fn get_json(&self, path: &str) -> Result<Value, CxdbClientError> {
        let response = self
            .client
            .get(self.endpoint(path))
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http get failed: {err}")))?;
        self.expect_ok_json(response).await
    }

    async fn get_text(&self, path: &str) -> Result<String, CxdbClientError> {
        let response = self
            .client
            .get(self.endpoint(path))
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http get failed: {err}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http read body failed: {err}")))?;
        if !status.is_success() {
            return Err(CxdbClientError::Backend(format!(
                "http request failed with status {status}: {text}"
            )));
        }
        Ok(text)
    }

    async fn post_json(&self, path: &str, body: Value) -> Result<Value, CxdbClientError> {
        let response = self
            .client
            .post(self.endpoint(path))
            .json(&body)
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http post failed: {err}")))?;
        self.expect_ok_json(response).await
    }

    async fn put_json(&self, path: &str, body: Value) -> Result<(), CxdbClientError> {
        let response = self
            .client
            .put(self.endpoint(path))
            .json(&body)
            .send()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http put failed: {err}")))?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable body>".to_string());
        Err(CxdbClientError::Backend(format!(
            "http put failed with status {status}: {body}"
        )))
    }

    async fn expect_ok_json(&self, response: reqwest::Response) -> Result<Value, CxdbClientError> {
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http read body failed: {err}")))?;
        if !status.is_success() {
            return Err(CxdbClientError::Backend(format!(
                "http request failed with status {status}: {text}"
            )));
        }
        serde_json::from_str(&text)
            .map_err(|err| CxdbClientError::Backend(format!("http json decode failed: {err}")))
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

    fn parse_hash_hex(payload: &Value, key: &str) -> Option<[u8; 32]> {
        let raw = payload.get(key).and_then(Value::as_str)?;
        parse_hex_32(raw)
    }

    fn decode_turn_payload(turn: &Value) -> Result<Vec<u8>, CxdbClientError> {
        if let Some(bytes_b64) = turn.get("bytes_b64").and_then(Value::as_str) {
            return base64::engine::general_purpose::STANDARD
                .decode(bytes_b64)
                .map_err(|err| {
                    CxdbClientError::Backend(format!("bytes_b64 decode failed: {err}"))
                });
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

    fn to_http_turn(turn: &Value, context_id: u64) -> Result<HttpStoredTurn, CxdbClientError> {
        let turn_id = Self::parse_u64_field(turn, "turn_id")?;
        let parent_turn_id = Self::parse_u64_field(turn, "parent_turn_id")?;
        let depth = Self::parse_u32_field(turn, "depth")?;
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
        let payload = Self::decode_turn_payload(turn)?;
        let content_hash = Self::parse_hash_hex(turn, "content_hash_b3")
            .or_else(|| Self::parse_hash_hex(turn, "content_hash"))
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
}

fn binary_addr_from_env() -> String {
    std::env::var("FORGE_CXDB_BINARY_ADDR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("CXDB_ADDR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .or_else(|| {
            std::env::var("CXDB_BINARY_ADDR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_CXDB_BINARY_ADDR.to_string())
}

#[derive(Clone)]
struct LiveHarness {
    store: CxdbStoreAdapter<LiveHttpClient, LiveHttpClient>,
    supports_context_write_routes: bool,
}

#[async_trait]
impl CxdbBinaryClient for LiveHttpClient {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let body = json!({"base_turn_id": base_turn_id.to_string()});
        let payload = self.post_json("/v1/contexts/create", body).await?;
        Ok(BinaryContextHead {
            context_id: Self::parse_u64_field(&payload, "context_id")?,
            head_turn_id: Self::parse_u64_field(&payload, "head_turn_id")?,
            head_depth: Self::parse_u32_field(&payload, "head_depth")?,
        })
    }

    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let body = json!({"base_turn_id": from_turn_id.to_string()});
        let payload = self.post_json("/v1/contexts/fork", body).await?;
        Ok(BinaryContextHead {
            context_id: Self::parse_u64_field(&payload, "context_id")?,
            head_turn_id: Self::parse_u64_field(&payload, "head_turn_id")?,
            head_depth: Self::parse_u32_field(&payload, "head_depth")?,
        })
    }

    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(&request.payload);
        let data = json!({"payload_b64": payload_b64});
        let body = json!({
            "type_id": request.type_id,
            "type_version": request.type_version,
            "data": data,
            "parent_turn_id": request.parent_turn_id.to_string(),
            "idempotency_key": request.idempotency_key,
        });
        let path = format!("/v1/contexts/{}/append", request.context_id);
        let payload = self.post_json(&path, body).await?;

        Ok(BinaryAppendTurnResponse {
            context_id: Self::parse_u64_field(&payload, "context_id")?,
            new_turn_id: Self::parse_u64_field(&payload, "turn_id")?,
            new_depth: Self::parse_u32_field(&payload, "depth")?,
            content_hash: Self::parse_hash_hex(&payload, "content_hash")
                .unwrap_or(request.content_hash),
        })
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let payload = self.get_json(&format!("/v1/contexts/{context_id}")).await?;
        Ok(BinaryContextHead {
            context_id: Self::parse_u64_field(&payload, "context_id")?,
            head_turn_id: Self::parse_u64_field(&payload, "head_turn_id")?,
            head_depth: Self::parse_u32_field(&payload, "head_depth")?,
        })
    }

    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        _include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        let payload = self
            .get_json(&format!(
                "/v1/contexts/{context_id}/turns?limit={limit}&view=both"
            ))
            .await?;
        let turns = payload
            .get("turns")
            .and_then(Value::as_array)
            .ok_or_else(|| CxdbClientError::Backend("missing turns array".to_string()))?;

        turns
            .iter()
            .map(|turn| {
                let parsed = LiveHttpClient::to_http_turn(turn, context_id)?;
                Ok(BinaryStoredTurn {
                    context_id: parsed.context_id,
                    turn_id: parsed.turn_id,
                    parent_turn_id: parsed.parent_turn_id,
                    depth: parsed.depth,
                    type_id: parsed.type_id,
                    type_version: parsed.type_version,
                    payload: parsed.payload,
                    idempotency_key: parsed.idempotency_key,
                    content_hash: parsed.content_hash,
                })
            })
            .collect()
    }

    async fn put_blob(&self, _raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "live tests do not implement blob PUT over HTTP".to_string(),
        ))
    }

    async fn get_blob(&self, _content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "live tests do not implement blob GET over HTTP".to_string(),
        ))
    }

    async fn attach_fs(
        &self,
        _turn_id: u64,
        _fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        Err(CxdbClientError::Backend(
            "live tests do not implement ATTACH_FS over HTTP".to_string(),
        ))
    }
}

#[async_trait]
impl CxdbHttpClient for LiveHttpClient {
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
        let payload = self.get_json(&path).await?;
        let turns = payload
            .get("turns")
            .and_then(Value::as_array)
            .ok_or_else(|| CxdbClientError::Backend("missing turns array".to_string()))?;

        turns
            .iter()
            .map(|turn| LiveHttpClient::to_http_turn(turn, context_id))
            .collect()
    }

    async fn publish_registry_bundle(
        &self,
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        let body: Value = serde_json::from_slice(bundle_json).map_err(|err| {
            CxdbClientError::InvalidInput(format!("registry bundle must be JSON bytes: {err}"))
        })?;
        self.put_json(&format!("/v1/registry/bundles/{bundle_id}"), body)
            .await
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
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(CxdbClientError::Backend(format!(
                "http get failed with status {status}: {body}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|err| CxdbClientError::Backend(format!("http read body failed: {err}")))?;
        Ok(Some(bytes.to_vec()))
    }
}

fn parse_hex_32(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in input.as_bytes().chunks_exact(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[index] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

fn unique_bundle_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("forge-live-{}-{}", now.as_secs(), now.subsec_nanos())
}

fn registry_bundle_bytes(bundle_id: &str, type_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "registry_version": 1,
        "bundle_id": bundle_id,
        "types": {
            type_id: {
                "versions": {
                    "1": {
                        "fields": {
                            "1": {"name": "payload_b64", "type": "string"}
                        }
                    }
                }
            }
        }
    }))
    .expect("bundle json should serialize")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LiveTypedPayload {
    value: String,
}

async fn build_live_harness() -> LiveHarness {
    let mut client = LiveHttpClient::from_env();
    let _ = client
        .get_text("/healthz")
        .await
        .expect("failed to reach CXDB HTTP endpoint '/healthz'; set CXDB_HTTP_BASE_URL to the CXDB HTTP API endpoint");
    let _ = client
        .get_json("/v1/contexts")
        .await
        .expect("failed to reach CXDB HTTP endpoint '/v1/contexts'");

    let supports_context_write_routes = client
        .client
        .post(client.endpoint("/v1/contexts/create"))
        .json(&json!({"base_turn_id":"0"}))
        .send()
        .await
        .map(|response| response.status() != reqwest::StatusCode::NOT_FOUND)
        .unwrap_or(false);
    client.supports_context_write_routes = supports_context_write_routes;

    LiveHarness {
        store: CxdbStoreAdapter::new(client.clone(), client),
        supports_context_write_routes,
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_health_and_metrics_against_running_cxdb() {
    let client = LiveHttpClient::from_env();
    let healthz = client
        .get_text("/healthz")
        .await
        .expect("healthz endpoint should be reachable");
    assert_eq!(healthz.trim(), "ok");

    let metrics = client
        .get_json("/v1/metrics")
        .await
        .expect("metrics endpoint should be reachable");
    assert!(metrics.is_object(), "expected /v1/metrics JSON object");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_create_append_list_and_paging_against_running_cxdb() {
    let harness = build_live_harness().await;
    if !harness.supports_context_write_routes {
        eprintln!(
            "skipping: this CXDB HTTP API build does not expose context create/append routes; write-path coverage requires binary protocol"
        );
        return;
    }
    let store = harness.store;

    let bundle_id = unique_bundle_id();
    let type_id = "forge.test.live_payload";
    let bundle_json = registry_bundle_bytes(&bundle_id, type_id);
    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id: bundle_id.clone(),
            bundle_json,
        })
        .await
        .expect("registry bundle publish should succeed");

    let context = store
        .create_context(None)
        .await
        .expect("context create should succeed");

    let t1 = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: b"first".to_vec(),
            idempotency_key: "live-k1".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("append 1 should succeed");
    let t2 = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: b"second".to_vec(),
            idempotency_key: "live-k2".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("append 2 should succeed");

    let head = store
        .get_head(&context.context_id)
        .await
        .expect("head lookup should succeed");
    assert_eq!(head.turn_id, t2.turn_id);

    let turns = store
        .list_turns(&context.context_id, None, 10)
        .await
        .expect("list should succeed");
    assert!(turns.len() >= 2);
    assert_eq!(turns[turns.len() - 2].turn_id, t1.turn_id);
    assert_eq!(turns[turns.len() - 1].turn_id, t2.turn_id);

    let older = store
        .list_turns(&context.context_id, Some(&t2.turn_id), 10)
        .await
        .expect("paged list should succeed");
    assert!(older.iter().any(|turn| turn.turn_id == t1.turn_id));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_binary_create_append_list_and_head_against_running_cxdb() {
    let http_client = LiveHttpClient::from_env();
    let _ = http_client
        .get_text("/healthz")
        .await
        .expect("healthz endpoint should be reachable");

    let binary_addr = binary_addr_from_env();
    let binary_client = CxdbSdkBinaryClient::connect(&binary_addr).unwrap_or_else(|error| {
        panic!("failed to connect CXDB binary endpoint at {binary_addr}: {error}")
    });
    let store = CxdbStoreAdapter::new(binary_client, http_client.clone());

    let bundle_id = unique_bundle_id();
    let type_id = "forge.test.live_binary_payload";
    let bundle_json = registry_bundle_bytes(&bundle_id, type_id);
    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id,
            bundle_json,
        })
        .await
        .expect("registry bundle publish should succeed");

    let context = store
        .create_context(None)
        .await
        .expect("context create over binary should succeed");

    let t1 = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: rmp_serde::to_vec_named(&json!({ "value": "first" }))
                .expect("msgpack payload should encode"),
            idempotency_key: "live-binary-k1".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("append 1 over binary should succeed");
    let t2 = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: rmp_serde::to_vec_named(&json!({ "value": "second" }))
                .expect("msgpack payload should encode"),
            idempotency_key: "live-binary-k2".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("append 2 over binary should succeed");

    let head = store
        .get_head(&context.context_id)
        .await
        .expect("head lookup over binary should succeed");
    assert_eq!(head.turn_id, t2.turn_id);

    let turns = store
        .list_turns(&context.context_id, None, 10)
        .await
        .expect("list should succeed");
    assert!(turns.iter().any(|turn| turn.turn_id == t1.turn_id));
    assert!(turns.iter().any(|turn| turn.turn_id == t2.turn_id));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_typed_projection_list_expected_decoded_records() {
    let harness = build_live_harness().await;
    if !harness.supports_context_write_routes {
        eprintln!(
            "skipping: this CXDB HTTP API build does not expose context create/append routes; write-path coverage requires binary protocol"
        );
        return;
    }
    let store = harness.store;

    let bundle_id = unique_bundle_id();
    let type_id = "forge.test.live_typed_projection_payload";
    let bundle_json = registry_bundle_bytes(&bundle_id, type_id);
    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id,
            bundle_json,
        })
        .await
        .expect("registry bundle publish should succeed");

    let context = store
        .create_context(None)
        .await
        .expect("context create should succeed");

    store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: serde_json::to_vec(&LiveTypedPayload {
                value: "typed".to_string(),
            })
            .expect("json encode should succeed"),
            idempotency_key: "live-typed-projection-k1".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("append should succeed");

    let records = store
        .list_typed_records::<LiveTypedPayload>(&context.context_id, None, 8)
        .await
        .expect("typed list should succeed");
    assert!(!records.is_empty());
    assert_eq!(records[records.len() - 1].1.value, "typed");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_idempotency_against_running_cxdb_returns_existing_turn() {
    let harness = build_live_harness().await;
    if !harness.supports_context_write_routes {
        eprintln!(
            "skipping: this CXDB HTTP API build does not expose context create/append routes; write-path coverage requires binary protocol"
        );
        return;
    }
    let store = harness.store;

    let bundle_id = unique_bundle_id();
    let type_id = "forge.test.live_payload";
    let bundle_json = registry_bundle_bytes(&bundle_id, type_id);
    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id,
            bundle_json,
        })
        .await
        .expect("registry bundle publish should succeed");

    let context = store
        .create_context(None)
        .await
        .expect("context create should succeed");

    let first = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: b"same".to_vec(),
            idempotency_key: "live-idempotency-key".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("first append should succeed");

    let second = store
        .append_turn(AppendTurnRequest {
            context_id: context.context_id,
            parent_turn_id: None,
            type_id: type_id.to_string(),
            type_version: 1,
            payload: b"same".to_vec(),
            idempotency_key: "live-idempotency-key".to_string(),
            fs_root_hash: None,
        })
        .await
        .expect("second append should succeed");

    assert_eq!(first.turn_id, second.turn_id);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live test; requires running CXDB instance"]
async fn live_registry_roundtrip_against_running_cxdb() {
    let harness = build_live_harness().await;
    let store = harness.store;

    let bundle_id = unique_bundle_id();
    let type_id = "forge.test.live_payload";
    let bundle_json = registry_bundle_bytes(&bundle_id, type_id);
    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id: bundle_id.clone(),
            bundle_json,
        })
        .await
        .expect("registry bundle publish should succeed");

    let fetched = store
        .get_registry_bundle(&bundle_id)
        .await
        .expect("registry get should succeed")
        .expect("registry bundle should exist");

    let fetched_json: Value =
        serde_json::from_slice(&fetched).expect("registry response should be valid JSON");
    assert_eq!(
        fetched_json
            .get("bundle_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        bundle_id
    );
}
