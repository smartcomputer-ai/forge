# forge-turnstore-cxdb

CXDB-backed implementation of Forge turnstore interfaces.

## Scope

This crate provides:
- `TurnStore` mapping to CXDB binary + HTTP operations
- `TypedTurnStore` mapping to CXDB registry HTTP APIs
- `ArtifactStore` mapping to CXDB blob/file attachment binary APIs

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
- Cursor paging and typed projection reads should use HTTP APIs.
- Registry bundle publication should happen before first writes of new schema versions.

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

`forge-turnstore-cxdb` stores immutable turns/blobs and does not enforce retention directly.
Retention and purge policies should be implemented by the CXDB deployment layer and host controls.

## Troubleshooting

- `invalid input: context_id must be a u64-compatible string`: ensure opaque Forge IDs are numeric strings at CXDB boundary.
- `resource not found: context (...)`: create/fork context before append or verify context lifecycle.
- `conflict`/idempotency mismatches: verify deterministic idempotency key construction and context scoping.
- Missing projection rows when paging: verify HTTP path is used for `before_turn_id` queries.

## Cross-check references

- `spec/cxdb/protocol.md`
- `spec/cxdb/http-api.md`
- `spec/cxdb/architecture.md`
- `spec/cxdb/type-registry.md`
