# forge-cli

`forge-cli` is the in-process host surface for Forge runtime execution (`run`,
`resume`, `inspect-checkpoint`) with CXDB-aware persistence wiring.

## CXDB read/write surfaces

- Binary (`FORGE_CXDB_BINARY_ADDR`) is used for write-heavy runtime operations.
- HTTP (`FORGE_CXDB_HTTP_BASE_URL`) is used for typed turn listing/projection
  and registry bundle APIs.

## Persistence mode

- `FORGE_CXDB_PERSISTENCE=off`: skip CXDB persistence writes.
- `FORGE_CXDB_PERSISTENCE=required`: fail run/session if CXDB persistence
  operations fail.

## Operational notes

- Keep binary endpoints private and protected with TLS/network controls.
- Put HTTP endpoints behind authenticated gateways.
- Ensure Forge registry bundles are published before new schema-version writes:
  - `forge.agent.runtime.v2`
  - `forge.attractor.runtime.v2`
