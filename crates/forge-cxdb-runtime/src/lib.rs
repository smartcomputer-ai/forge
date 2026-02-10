#![doc = r#"
CXDB runtime integration contracts and clients.

Operation mapping (spec cross-check):

| Runtime method | CXDB API | Spec section |
| --- | --- | --- |
| `CxdbRecordStore::create_context` | binary `CTX_CREATE` | `spec/cxdb/protocol.md` "2. CTX_CREATE" |
| `CxdbRecordStore::fork_context` | binary `CTX_FORK` | `spec/cxdb/protocol.md` "3. CTX_FORK" |
| `CxdbRecordStore::append_turn` | binary `APPEND_TURN` | `spec/cxdb/protocol.md` "5. APPEND_TURN" |
| `CxdbRecordStore::get_head` | binary `GET_HEAD` | `spec/cxdb/protocol.md` "4. GET_HEAD" |
| `CxdbRecordStore::list_turns` (no cursor) | binary `GET_LAST` | `spec/cxdb/protocol.md` "6. GET_LAST" |
| `CxdbRecordStore::list_turns` (`before_turn_id`) | HTTP `GET /v1/contexts/:id/turns` | `spec/cxdb/http-api.md` "Get Turns from Context" |
| `CxdbRegistryStore::publish_registry_bundle` | HTTP `PUT /v1/registry/bundles/:bundle_id` | `spec/cxdb/http-api.md` "Publish Registry Bundle" |
| `CxdbRegistryStore::get_registry_bundle` | HTTP `GET /v1/registry/bundles/:bundle_id` | `spec/cxdb/http-api.md` "Get Registry Bundle" |
| `CxdbArtifactClient::put_blob` | binary `PUT_BLOB` | `spec/cxdb/protocol.md` "9. PUT_BLOB" |
| `CxdbArtifactClient::get_blob` | binary `GET_BLOB` | `spec/cxdb/protocol.md` "7. GET_BLOB" |
| `CxdbArtifactClient::attach_fs` | binary `ATTACH_FS` | `spec/cxdb/protocol.md` "8. ATTACH_FS" |

Implementation notes:
- Forge IDs remain opaque `String` values and are converted to `u64` only at the adapter boundary.
- `append_turn` computes BLAKE3 content hash over uncompressed payload bytes.
- If `AppendTurnRequest.idempotency_key` is empty, the adapter generates a deterministic fallback key.
- Cursor paging (`before_turn_id`) is routed to HTTP so projection semantics stay aligned with CXDB.
"#]

pub mod adapter;
pub mod runtime;
pub mod testing;

pub use adapter::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbReqwestHttpClient, CxdbSdkBinaryClient,
    CxdbStoreAdapter, DEFAULT_CXDB_BINARY_ADDR, DEFAULT_CXDB_HTTP_BASE_URL, HttpStoredTurn,
};
pub use runtime::{
    AppendTurnRequest as CxdbAppendTurnRequest, BlobHash as CxdbBlobHash,
    ContextId as CxdbContextId, CxdbRuntimeStore, StoreContext as CxdbStoreContext,
    StoredTurn as CxdbStoredTurn, StoredTurnRef as CxdbStoredTurnRef, TurnId as CxdbTurnId,
};
pub use testing::MockCxdb;
