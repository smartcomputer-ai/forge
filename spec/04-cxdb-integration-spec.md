# CXDB Integration Specification (Extension)

This document defines how Forge integrates CXDB as the primary durable store for runtime history, lineage, and drill-down queries.

It is an extension spec, not a replacement for existing specs:
- `01-unified-llm-spec.md` remains the provider transport layer.
- `02-coding-agent-loop-spec.md` remains the agent runtime contract.
- `03-attractor-spec.md` remains the pipeline orchestration contract.

---

## Table of Contents

1. [Overview and Goals](#1-overview-and-goals)
2. [Layering and Boundaries](#2-layering-and-boundaries)
3. [Data Model Mapping](#3-data-model-mapping)
4. [Integration Contracts](#4-integration-contracts)
5. [Runtime Semantics](#5-runtime-semantics)
6. [Rollout Plan](#6-rollout-plan)
7. [Definition of Done](#7-definition-of-done)

---

## 1. Overview and Goals

### 1.1 Problem

After P32, Forge has a working CXDB adapter path, but current layering still routes runtime persistence through generic `forge-turnstore` abstractions.

That layering now creates avoidable leakage and ambiguity:
- runtime behavior is already CXDB-shaped,
- capability mismatches appear at trait boundaries,
- host/runtime wiring does extra conversions,
- critical semantics (idempotency, parent resolution, fs attachment, projection paging) are better expressed directly in CXDB terms.

### 1.2 Architecture Decision

Forge adopts a CXDB-first persistence architecture for runtime write and read paths.

Implications:
- `forge-agent` and `forge-attractor` persist runtime events via CXDB-facing contracts.
- `forge-llm` remains CXDB-independent.
- `forge-cxdb-runtime` is the runtime CXDB integration crate.
- legacy turnstore crates are removed from active runtime architecture.

### 1.3 Goals

This extension defines:
- direct CXDB runtime write-path contracts,
- projection-native read/query contracts,
- fs lineage using CXDB `fstree` and turn attachment,
- schema/registry discipline for typed projection compatibility,
- migration phases from turnstore abstraction to CXDB-first runtime cores.

### 1.4 Non-goals

- Replacing `forge-llm` request/response contracts.
- Guaranteeing multi-backend portability for persistent runtime storage.
- Requiring a live CXDB service for deterministic unit tests.

---

## 2. Layering and Boundaries

### 2.1 Layer Placement

CXDB integration belongs above the LLM transport layer and below host/UI surfaces:

- `forge-llm`: no direct CXDB dependency.
- `forge-agent`: optional runtime persistence using CXDB-facing contracts.
- `forge-attractor`: optional runtime persistence using CXDB-facing contracts.
- Host (CLI/HTTP/TUI/Web): configures endpoints and CXDB enablement and may consume CXDB projections.

### 2.2 Architecture Rules

Rules:
- Core runtime logic MUST remain deterministic with persistence disabled (`off`).
- Runtime persistence contracts MAY be CXDB-specific; they MUST NOT leak CXDB details into `forge-llm`.
- Conversion between runtime domain records and encoded CXDB payloads MUST happen at persistence boundaries, not scattered across business logic.
- Projection/query decoding logic SHOULD consume typed projection responses instead of ad hoc JSON envelope decoding when typed projection is available.

### 2.3 Crate Topology Policy

Current workspace includes:
- `crates/forge-cxdb` (vendored CXDB client and fstree helpers),
- `crates/forge-cxdb-runtime` (CXDB runtime integration contracts and deterministic fakes).

Target direction:
- runtime cores (`forge-agent`, `forge-attractor`) use CXDB-first persistence contracts,
- `forge-cxdb-runtime` remains the shared runtime CXDB boundary,
- host/bootstrap layers own endpoint/wiring policy.

### 2.4 CXDB Protocol Usage Policy

By default:
- write-heavy runtime path (create/fork/append/head/last/blob/fs attach): binary protocol (`:9009`),
- typed projection and cursor paging: HTTP API (`:9010`),
- registry bundle publish/read: HTTP API,
- HTTP-only CXDB connectivity is acceptable for bootstrap/testing but SHOULD NOT be the production default.

### 2.5 CXDB Cross-check References

Authoritative references in this repository:
- `crates/forge-cxdb/docs/protocol.md`
- `crates/forge-cxdb/README.md`

When vendored docs are extended, add cross-check references here for:
- HTTP projection and paging contracts,
- type registry bundle contract,
- storage/concurrency invariants.

### 2.6 Transition Posture

During migration (P33-P37):
- existing turnstore adapters MAY coexist,
- new runtime code SHOULD prefer CXDB-first contracts,
- new feature work MUST NOT deepen dependency on generic turnstore abstractions.

---

## 3. Data Model Mapping

### 3.1 Mapping Principles

- One CXDB context represents one logical execution thread.
- Every durable runtime event is an immutable CXDB turn.
- Parent-child turn relationships mirror causality.
- Branching/fan-out/fan-in are represented explicitly via context forks and linkage records.
- Attractor and agent timelines MUST be correlated by stable identifiers.

### 3.2 Agent Mapping (`02-coding-agent-loop-spec.md`)

Recommended mapping:
- agent session root -> CXDB context,
- `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SteeringTurn`, `SystemTurn` -> CXDB turns,
- subagent spawn -> forked context from triggering parent turn,
- session lifecycle events MAY be separate event turns.

Recommended `type_id` namespace:
- `forge.agent.user_turn`
- `forge.agent.assistant_turn`
- `forge.agent.tool_results_turn`
- `forge.agent.steering_turn`
- `forge.agent.system_turn`
- `forge.agent.event`
- `forge.link.subagent_spawn`

### 3.3 Attractor Mapping (`03-attractor-spec.md`)

Recommended mapping:
- pipeline run root -> CXDB context,
- stage lifecycle events -> turns,
- interview/human-gate lifecycle -> turns,
- checkpoint save -> turn with checkpoint pointer/hash and minimal state summary,
- route decisions and retries -> turns with explicit routing metadata,
- DOT source and normalized graph snapshot -> turns with inline payload and/or artifact refs.

Recommended `type_id` namespace:
- `forge.attractor.run_event`
- `forge.attractor.stage_event`
- `forge.attractor.interview_event`
- `forge.attractor.checkpoint_event`
- `forge.attractor.route_decision`
- `forge.attractor.dot_source`
- `forge.attractor.graph_snapshot`
- `forge.link.stage_to_agent`

### 3.4 Cross-layer Correlation Requirements

Persisted records SHOULD include:
- `run_id`
- `pipeline_context_id`
- `node_id`
- `stage_attempt_id`
- `agent_session_id` (if stage invokes agent)
- `agent_context_id`
- `agent_head_turn_id` (optional)
- `parent_turn_id`
- `sequence_no` (monotonic within logical stream)

A `forge.link.stage_to_agent` record MUST be emitted when a stage creates or attaches to an agent session.

### 3.5 Envelope Requirements

Persisted runtime payloads SHOULD use a stable envelope:

```
RECORD StoredTurnEnvelope:
    schema_version    : Integer
    event_kind        : String
    timestamp         : Timestamp
    correlation       : Object
    payload           : Object
```

Rules:
- `schema_version` MUST be present.
- `correlation` MUST include enough keys for attractor <-> agent traversal.
- `payload` SHOULD avoid non-deterministic fields unless explicitly marked diagnostic.
- large data SHOULD be referenced as artifacts (`blob_hash`, `size`, optional `mime_type`) instead of inlining all bytes.

Forge P36 baseline (v1 runtime envelope wire format):
- envelope bytes are msgpack with stable numeric tags (write path no longer persists JSON envelope bytes),
- required envelope tags:
  - `1 schema_version`
  - `2 run_id`
  - `3 session_id`
  - `4 node_id`
  - `5 stage_attempt_id`
  - `6 event_kind`
  - `7 timestamp`
  - `8 payload_json` (JSON string form of payload object)
  - `9..18` correlation fields (`corr_run_id`, `corr_pipeline_context_id`, `corr_node_id`, `corr_stage_attempt_id`, `corr_agent_session_id`, `corr_agent_context_id`, `corr_agent_head_turn_id`, `corr_parent_turn_id`, `corr_sequence_no`, `corr_thread_key`)

### 3.6 Filesystem Lineage Requirements (`fstree`)

For workspace lineage, records SHOULD include:
- `fs_root_hash` (BLAKE3-256 hash of Merkle root),
- `snapshot_policy_id`,
- optional snapshot stats (`file_count`, `dir_count`, `symlink_count`, `total_bytes`, `bytes_uploaded`).

Snapshot policy MUST define:
- include/exclude patterns,
- symlink behavior (`follow_symlinks` false by default),
- limits (`max_files`, `max_file_size`),
- capture boundary (for example stage start/end/checkpoint).

### 3.7 Encoding and Registry Contract

For typed projections:
- each turn MUST include `type_id` and `type_version`,
- Forge-owned schemas SHOULD use stable numeric msgpack tags,
- runtime writers SHOULD encode deterministic msgpack bytes directly,
- registry bundles SHOULD be published before or alongside first writes for new schema versions,
- unknown tags/fields MUST be forward-compatible for readers.

Runtime bundle publication policy:
- Forge runtime bootstrap paths SHOULD publish the relevant Forge bundle (`forge.agent.runtime.v1`, `forge.attractor.runtime.v1`) before first append for `required` mode sessions/runs.
- bundle publication SHOULD be idempotent (safe to retry and safe when already present).
- field-tag evolution MUST follow type-registry rules (never reuse tags; bump `type_version` on descriptor changes).

### 3.8 Branch Context Policy

- Agent: one context per session thread; each subagent gets a forked context.
- Attractor: parallel fan-out branches SHOULD run in forked contexts from a pre-branch turn.
- Fan-in turns SHOULD reference all source branch contexts and terminal turn IDs.

---

## 4. Integration Contracts

### 4.1 Runtime Write Contract (CXDB-first)

Runtime persistence SHOULD expose CXDB-shaped operations:

```
TYPE ContextId = String            // Opaque in Forge, u64-backed in CXDB
TYPE TurnId = String               // Opaque in Forge, u64-backed in CXDB
TYPE BlobHash = String             // Lowercase hex BLAKE3-256

RECORD CxdbAppendRequest:
    context_id       : ContextId
    parent_turn_id   : TurnId | None
    type_id          : String
    type_version     : Integer
    payload          : Bytes
    idempotency_key  : String
    fs_root_hash     : BlobHash | None

INTERFACE CxdbRuntimeWriter:
    FUNCTION create_context(base_turn_id: TurnId | None) -> ContextHead
    FUNCTION fork_context(from_turn_id: TurnId) -> ContextHead
    FUNCTION append_turn(request: CxdbAppendRequest) -> StoredTurn
    FUNCTION get_head(context_id: ContextId) -> StoredTurnRef
    FUNCTION get_last(context_id: ContextId, limit: Integer, include_payload: Bool) -> List<StoredTurn>
```

### 4.2 Projection Read Contract

Projection/query surfaces SHOULD expose:

```
INTERFACE CxdbProjectionReader:
    FUNCTION list_turns(context_id: ContextId, before_turn_id: TurnId | None, limit: Integer) -> List<StoredTurn>
    FUNCTION publish_registry_bundle(bundle_id: String, bundle_json: Bytes) -> Void
    FUNCTION get_registry_bundle(bundle_id: String) -> Bytes | None
```

Rule:
- query/list surfaces SHOULD use HTTP typed projection APIs by default (with or without `before_turn_id`).

### 4.3 Artifact and FS Contract

```
INTERFACE CxdbArtifactClient:
    FUNCTION put_blob(raw_bytes: Bytes) -> BlobHash
    FUNCTION get_blob(content_hash: BlobHash) -> Bytes | None
    FUNCTION attach_fs(turn_id: TurnId, fs_root_hash: BlobHash) -> Void
```

Notes:
- `append_turn_with_fs` is preferred when fs root is available at append time.
- `attach_fs` is valid for post-hoc attachment.
- if a client implementation does not yet expose `get_blob`, runtime/host code MUST degrade gracefully and clearly signal unsupported retrieval behavior.

### 4.4 Runtime Hook Points

`forge-agent` SHOULD persist at:
- session start/end,
- input acceptance,
- assistant completion,
- tool call start/end,
- steering injection,
- subagent spawn/close linkage,
- optional checkpoint snapshots.

`forge-attractor` SHOULD persist at:
- run start/finalization,
- stage start/end/failure/retry,
- edge selection decision,
- human-gate lifecycle,
- checkpoint save,
- stage-to-agent linkage creation,
- dot source and normalized graph snapshot at run initialization.

### 4.5 CXDB Persistence Toggle

Runtime config SHOULD support:
- `off`: skip persistence writes,
- `required`: fail run/session when persistence write fails.

Recommended defaults:
- local deterministic test runs: `off` unless CXDB-specific behavior is under test,
- CXDB-enabled runs: `required`.

### 4.6 Idempotency and Parent Semantics

Rules:
- append retries MUST use deterministic idempotency keys,
- keys SHOULD be derived from stable correlation fields and sequence numbers,
- parent resolution behavior MUST be explicit (`parent_turn_id` if present, else current head),
- returned turn metadata MUST reflect the committed parent semantics.

Example key patterns:
- attractor: `run_id + node_id + stage_attempt_id + event_kind + sequence_no`,
- agent: `session_id + local_turn_index + event_kind`.

### 4.7 CXDB Operation Mapping Contract

Expected mapping:
- `create_context` -> `CTX_CREATE`
- `fork_context` -> `CTX_FORK`
- `append_turn` -> `APPEND_TURN` (or `append_turn_with_fs` flag path)
- `get_head` -> `GET_HEAD`
- `get_last` newest-window reads -> `GET_LAST`
- `list_turns(before_turn_id)` -> HTTP turn listing/paging API
- registry publish/read -> HTTP registry APIs
- artifact/fs operations -> `PUT_BLOB`, `GET_BLOB`, `ATTACH_FS`

Adapters SHOULD keep Forge IDs opaque and only convert at CXDB boundaries.

### 4.8 FSTree Sync Flow

Preferred snapshot flow:
1. capture workspace snapshot via `fstree::capture(root, options)`
2. upload tree/file/symlink blobs via `snapshot.upload(ctx, client)`
3. append turn with `fs_root_hash` (preferred) or attach after append
4. store lineage metadata (`fs_root_hash`, policy, stats) in envelope payload

Rules:
- snapshot tree entries are sorted by name for deterministic hashing,
- file and tree objects are content-addressed and deduplicated,
- symlink loops MUST fail fast (`CyclicLink`),
- oversize/overcount policy violations MUST return explicit errors (`FileTooLarge`, `TooManyFiles`).

### 4.9 Deterministic Test Doubles

Because runtime architecture is CXDB-first, deterministic tests SHOULD use fake/mocked CXDB contracts instead of backend portability tests.

Minimum test tiers:
- unit tests with fake CXDB writer/reader,
- integration tests with deterministic in-process fakes,
- optional live CXDB tests gated by env vars.

---

## 5. Runtime Semantics

### 5.1 Source of Truth by Mode

- `off`: runtime memory/filesystem state is authoritative.
- `required`: CXDB write success is part of runtime correctness contract.

### 5.2 Workspace and Artifacts

Runtime execution still uses a filesystem workspace for tools and local reproducibility.

CXDB persistence SHOULD capture:
- small metadata directly in turn payloads,
- large artifacts via blob refs,
- full workspace lineage via fs root attachment at configured boundaries.

### 5.3 Ordering and Retry Guarantees

- writes MUST preserve causal order per context,
- retries MUST remain idempotent,
- cross-context causal links MUST be explicit in payload metadata,
- sequence number assignment MUST remain deterministic for repeated runs with the same execution path.

### 5.4 Branching and Fan-in

- subagents and parallel branches SHOULD fork from explicit pre-branch turns,
- fan-in/merge events MUST reference all contributing branch contexts,
- branch lineage MUST remain queryable through correlation metadata.

### 5.5 Privacy and Retention

Integrations MUST allow:
- payload redaction for secrets,
- retention/TTL policy configuration,
- per-project persistence disablement.

### 5.6 Security Posture

- binary protocol endpoints are trusted-network surfaces and MUST be protected accordingly,
- HTTP projection surfaces SHOULD be behind authenticated gateways,
- TLS (or equivalent network controls) SHOULD be used in production,
- sensitive fields SHOULD be redacted before persistence where policy requires.

### 5.7 Renderer Boundary

Renderer loading/execution is host/UI scope, not core runtime scope:
- core libraries persist typed turns and emit events,
- host surfaces map `type_id` to renderer behavior,
- remote renderer execution (if any) MUST be host-policy-controlled.

### 5.8 Operational Runbook Requirements

Runtime and host documentation MUST include explicit runbooks for the following failure classes.

Append path failures (`CTX_CREATE`, `CTX_FORK`, `APPEND_TURN`, `GET_HEAD`):
- verify binary endpoint route, ACL, and TLS trust path,
- validate persistence mode policy (`off` vs `required`) against expected fail-open/fail-closed behavior,
- verify deterministic idempotency key generation and parent-turn resolution,
- verify context lifecycle ordering before append.

Projection path failures (HTTP turn listing/paging):
- verify HTTP gateway auth/policy and upstream route health,
- validate `context_id`, `before_turn_id`, and `limit` semantics,
- distinguish projection lag from ingest failure by cross-checking binary write success.

Registry mismatch (bundle decode or schema drift):
- verify bundle presence/readability for expected `bundle_id`,
- verify writer `type_id`/`type_version` alignment with published bundle versions,
- publish bundle before first write of new schema versions.

Filesystem snapshot/attachment failures (`fstree`, `PUT_BLOB`, `ATTACH_FS`):
- validate capture policy bounds and excludes,
- verify blob upload success before fs attachment,
- verify `fs_root_hash` integrity and turn linkage consistency.

---

## 6. Rollout Plan

### Phase A (P33): Architecture pivot and spec rebaseline
- adopt CXDB-first terminology and contracts,
- define crate-boundary and migration policy,
- establish turnstore sunset posture.

### Phase B (P34): Direct runtime write-path migration
- migrate agent and attractor writes from turnstore traits to CXDB-first contracts,
- preserve `off`/`required` semantics behind a CXDB enablement toggle,
- fix idempotency/parent-resolution correctness gaps.

### Phase C (P38): FSTree and workspace snapshot integration
- integrate deterministic snapshot capture/upload,
- attach fs roots on configured boundaries,
- normalize artifact model for blob refs plus fs lineage.

### Phase D (P36): Typed projection and query-surface refactor
- enforce msgpack numeric-tag writer discipline,
- publish/use registry bundles,
- migrate query/drill-down surfaces to typed projection APIs.

### Phase E (P37): Turnstore sunset and CXDB hardening
- remove or hard-deprecate turnstore runtime dependencies,
- finish operational runbooks and migration DoD matrix,
- rebaseline deferred roadmap work on CXDB-first foundation.

---

## 7. Definition of Done

### 7.1 Architecture
- [ ] `forge-agent` and `forge-attractor` runtime persistence paths are CXDB-first.
- [ ] `forge-llm` remains CXDB-independent.
- [ ] Turnstore abstraction is no longer a required runtime boundary.
- [ ] Repository docs and terminology reflect CXDB-first architecture.

### 7.2 Runtime Write Path
- [ ] Agent session and Attractor run/stage/checkpoint/link writes use CXDB-first contracts.
- [ ] CXDB persistence toggle behavior (`off`, `required`) is preserved and tested.
- [ ] Deterministic idempotency keys and committed parent semantics are validated.

### 7.3 FS Lineage
- [ ] Snapshot capture policy is explicit and configurable.
- [ ] Relevant turns include fs root attachment/lineage metadata.
- [ ] FSTree error modes and limits are deterministic and covered by tests.

### 7.4 Typed Projection and Query
- [ ] Runtime payloads are projection-ready msgpack with stable schema identifiers.
- [ ] Registry bundle lifecycle is documented and implemented.
- [ ] Host query surfaces use typed projection APIs with deterministic paging.

### 7.5 Operations and Security
- [ ] Endpoint topology and trust boundaries are documented.
- [ ] Redaction/retention controls are enforced by policy.
- [ ] Deterministic fake-CXDB and optional live-CXDB suites are green.
- [ ] Migration phases P33-P38 have an explicit closure matrix.
