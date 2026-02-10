# forge-cxdb-runtime

CXDB-backed runtime contracts and adapter surfaces for Forge.

## Scope

This crate provides:
- runtime context/turn contracts over CXDB binary + HTTP operations
- registry bundle publication/read helpers over CXDB HTTP APIs
- blob/file attachment helpers over CXDB binary APIs

## Endpoint topology

Recommended deployment topology:
- Binary endpoint (`:9009`): write-heavy runtime path
  - `CTX_CREATE`, `CTX_FORK`, `APPEND_TURN`, `GET_HEAD`, `GET_LAST`
  - `PUT_BLOB`, `GET_BLOB`, `ATTACH_FS`
- HTTP endpoint (`:9010`): projection and registry path
  - `GET /v1/contexts/:id/turns` with `before_turn_id` cursor paging
  - `PUT/GET /v1/registry/bundles/:bundle_id`

## Operational guidance

- Runtime writes should prefer binary protocol for throughput.
- Turn listing and typed projection reads should use HTTP APIs.
- Registry bundle publication should happen before first writes of new schema versions.
- Forge runtime bundles used by current bootstrap paths:
  - `forge.agent.runtime.v1`
  - `forge.attractor.runtime.v1`

## Trust boundaries and transport security

- Treat CXDB binary endpoint as trusted-network-only.
- Use TLS and network isolation for binary transport in production.
- Put HTTP API behind authenticated gateway/proxy.
- Restrict direct endpoint exposure; prefer private service networking.

## Security controls

This adapter does not itself redact secrets. Redaction must be applied before turn append.

Recommended controls:
- Payload redaction hook before `append_turn`
- Environment/tenant-specific retention policy at storage layer
- Restricted access to registry and projection surfaces

## Retention and lifecycle hooks

`forge-cxdb-runtime` stores immutable turns/blobs and does not enforce retention directly.
Retention and purge policies should be implemented by the CXDB deployment layer and host controls.

## Troubleshooting

- `invalid input: context_id must be a u64-compatible string`: ensure opaque Forge IDs are numeric strings at CXDB boundary.
- `resource not found: context (...)`: create/fork context before append or verify context lifecycle.
- `conflict`/idempotency mismatches: verify deterministic idempotency key construction and context scoping.
- Missing projection rows when paging: verify HTTP path is used for `before_turn_id` queries.

## Live tests

Live integration tests are in `crates/forge-cxdb-runtime/tests/live.rs` and are ignored by default.

Run against default local CXDB (`http://127.0.0.1:9010`):

```bash
cargo test -p forge-cxdb-runtime --test live -- --ignored
```

Run against a custom endpoint:

```bash
CXDB_HTTP_BASE_URL=http://localhost:9010 cargo test -p forge-cxdb-runtime --test live -- --ignored
```

Note: some CXDB builds expose read/registry HTTP routes but not HTTP context create/append routes. In that case, write-path live tests are auto-skipped and registry coverage still runs.

## Cross-check references

- `spec/cxdb/protocol.md`
- `spec/cxdb/http-api.md`
- `spec/cxdb/architecture.md`
- `spec/cxdb/type-registry.md`
