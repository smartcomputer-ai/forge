#![doc = r#"
CXDB-backed adapter for `forge-turnstore` traits.

Operation mapping (spec cross-check):

| Forge trait method | CXDB API | Spec section |
| --- | --- | --- |
| `TurnStore::create_context` | binary `CTX_CREATE` | `spec/cxdb/protocol.md` "2. CTX_CREATE" |
| `TurnStore::fork_context` | binary `CTX_FORK` | `spec/cxdb/protocol.md` "3. CTX_FORK" |
| `TurnStore::append_turn` | binary `APPEND_TURN` | `spec/cxdb/protocol.md` "5. APPEND_TURN" |
| `TurnStore::get_head` | binary `GET_HEAD` | `spec/cxdb/protocol.md` "4. GET_HEAD" |
| `TurnStore::list_turns` (no cursor) | binary `GET_LAST` | `spec/cxdb/protocol.md` "6. GET_LAST" |
| `TurnStore::list_turns` (`before_turn_id`) | HTTP `GET /v1/contexts/:id/turns` | `spec/cxdb/http-api.md` "Get Turns from Context" |
| `TypedTurnStore::publish_registry_bundle` | HTTP `PUT /v1/registry/bundles/:bundle_id` | `spec/cxdb/http-api.md` "Publish Registry Bundle" |
| `TypedTurnStore::get_registry_bundle` | HTTP `GET /v1/registry/bundles/:bundle_id` | `spec/cxdb/http-api.md` "Get Registry Bundle" |
| `ArtifactStore::put_blob` | binary `PUT_BLOB` | `spec/cxdb/protocol.md` "9. PUT_BLOB" |
| `ArtifactStore::get_blob` | binary `GET_BLOB` | `spec/cxdb/protocol.md` "7. GET_BLOB" |
| `ArtifactStore::attach_fs` | binary `ATTACH_FS` | `spec/cxdb/protocol.md` "8. ATTACH_FS" |

Implementation notes:
- Forge IDs remain opaque `String` values and are converted to `u64` only at the adapter boundary.
- `append_turn` computes BLAKE3 content hash over uncompressed payload bytes.
- If `AppendTurnRequest.idempotency_key` is empty, the adapter generates a deterministic fallback key.
- Cursor paging (`before_turn_id`) is routed to HTTP so projection semantics stay aligned with CXDB.
"#]

pub mod adapter;

pub use adapter::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbReqwestHttpClient, CxdbSdkBinaryClient,
    CxdbTurnStore, DEFAULT_CXDB_BINARY_ADDR, DEFAULT_CXDB_HTTP_BASE_URL, HttpStoredTurn,
};
