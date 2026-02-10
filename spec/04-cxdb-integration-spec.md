# CXDB Integration Specification (Extension)

This document defines how Forge integrates CXDB as a durable, branch-friendly turn store for agent and pipeline execution history.

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

Forge currently keeps runtime history primarily in in-memory structures and local run directories. This works for local execution but makes cross-run branching, external inspection, and long-lived replay/resume harder than necessary.

CXDB provides:
- immutable Turn DAG storage,
- O(1) branch/fork via context head pointers,
- content-addressed dedup for repeated payloads,
- typed projections for UI and tooling.

### 1.2 Goals

This extension adds an optional CXDB-backed persistence path for:
- agent conversation turns and tool outputs,
- attractor stage transitions, outcomes, routing, and human-gate decisions,
- cross-layer drill-down links (attractor stage -> agent session/thread -> agent turns/tools),
- DOT/graph artifacts and normalized graph snapshots for observability,
- checkpoint metadata for replay and resume.

### 1.3 Non-goals

- Replacing `forge-llm` request/response contracts.
- Coupling core loop correctness to CXDB availability by default.
- Requiring CXDB adapter implementation before Attractor runtime conformance milestones complete.

---

## 2. Layering and Boundaries

### 2.1 Layer Placement

CXDB integration belongs above the LLM transport layer and below hosts/UI:

- `forge-llm`: no direct CXDB dependency.
- `forge-agent`: optional turn persistence adapter.
- `forge-attractor` runtime: optional run/turn persistence adapter.
- Host (CLI/HTTP/TUI/Web): reads events from engines and may read projections from CXDB.

### 2.2 Architecture Rule

Core runtimes MUST depend on storage interfaces, not on CXDB SDK types.

The recommended crate split is:
- `forge-turnstore` (traits + shared record types + in-memory test implementation),
- optional filesystem backend as either `forge-turnstore-fs` or a `forge-turnstore` module,
- `forge-turnstore-cxdb` (CXDB implementation).

Attractor and agent MUST compile and run with `forge-turnstore` abstractions only.

### 2.3 Why this Layer

This placement preserves:
- deterministic core execution when storage is disabled,
- portability to other backing stores,
- fast deterministic tests via in-memory store,
- ability to defer CXDB transport work without rewriting runtime cores.

### 2.4 CXDB Protocol Usage

Integrations SHOULD use CXDB protocols by responsibility:

- Write-heavy runtime path (append/fork/head updates): prefer CXDB binary protocol (`:9009`) for throughput.
- Read/projection path (UI/tooling/timeline browsing): prefer CXDB HTTP API (`:9010`).
- HTTP-only write mode is acceptable for bootstrap/testing, but SHOULD NOT be default for production append paths.
- Binary protocol SHOULD also be used for blob/artifact upload paths (`PUT_BLOB`, `GET_BLOB`, `ATTACH_FS`) when enabled.
- Readers requiring typed projection or cursor paging (`before_turn_id`) SHOULD use HTTP endpoints even when writes use binary.

### 2.5 CXDB Cross-check References

For fast protocol verification while implementing:

- Binary wire operations and idempotency/compression behavior:
  `spec/cxdb/protocol.md` (Message Flows 2-10, Idempotency, Compression)
- HTTP paging/projection/registry behavior:
  `spec/cxdb/http-api.md` (Contexts, Turns, Registry, Blobs)
- Storage invariants and concurrency assumptions:
  `spec/cxdb/architecture.md` (Turn DAG, Blob CAS, Concurrency Model)
- Schema/tag evolution and typed projection rules:
  `spec/cxdb/type-registry.md` (Core Concepts, Registry Bundle Format, Schema Evolution)

---

## 3. Data Model Mapping

### 3.1 Mapping Principles

- One CXDB context represents one logical execution thread.
- Every emitted runtime event/turn can be represented as an immutable CXDB turn.
- Parent-child relationships mirror causality and branch points.
- Attractor turns and agent turns MUST be linkable by stable correlation fields.

### 3.2 Agent Mapping (`02-coding-agent-loop-spec.md`)

Recommended mapping:
- Agent session root -> new CXDB context.
- `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SteeringTurn`, `SystemTurn` -> CXDB turns.
- Session events MAY be persisted as separate event turns or embedded metadata.
- Subagent spawn -> forked CXDB context from the parent turn where spawn occurred.

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
- Pipeline run root -> new CXDB context.
- Stage lifecycle events -> turns (`StageStarted`, `StageCompleted`, `StageFailed`, etc.).
- Human interaction (`InterviewStarted`, `InterviewCompleted`, `InterviewTimeout`) -> turns.
- Checkpoint save -> turn containing checkpoint pointer/hash + minimal state summary.
- Retry/failure routing -> normal turns with explicit edge-selection metadata.

DOT and graph payload guidance:
- Each run SHOULD persist the source DOT payload (or immutable artifact reference + hash).
- Each run SHOULD persist the normalized graph snapshot used at execution start.
- Dot payloads MAY be large; payload dedup is expected through content addressing.

Recommended `type_id` namespace:
- `forge.attractor.stage_event`
- `forge.attractor.interview_event`
- `forge.attractor.checkpoint_event`
- `forge.attractor.route_decision`
- `forge.attractor.run_event`
- `forge.attractor.dot_source`
- `forge.attractor.graph_snapshot`
- `forge.link.stage_to_agent`

### 3.4 Cross-layer Correlation Requirements

For stage-level drill-down into agent behavior, persisted records SHOULD include:
- `run_id`
- `pipeline_context_id`
- `node_id`
- `stage_attempt_id`
- `agent_session_id` (when a stage invokes an agent)
- `agent_context_id` (turnstore context)
- `agent_head_turn_id` (optional, for quick jump)
- `parent_turn_id` (causal parent in context DAG)
- `sequence_no` (monotonic within logical stream)

A `forge.link.stage_to_agent` record MUST be emitted when a stage creates or attaches to an agent session.

### 3.5 Payload Requirements

Stored payloads SHOULD be stable, versioned envelopes:

```
RECORD StoredTurnEnvelope:
    schema_version    : Integer
    run_id            : String | None
    session_id        : String | None
    node_id           : String | None
    stage_attempt_id  : String | None
    event_kind        : String
    timestamp         : Timestamp
    payload           : Object
    correlation       : Object
```

Rules:
- `schema_version` MUST be present for migration safety.
- `correlation` MUST contain enough linkage to traverse attractor <-> agent timelines.
- `payload` MUST avoid non-deterministic fields unless explicitly marked diagnostic.
- Large blobs SHOULD be stored as artifacts with references in the envelope.
- Artifact references SHOULD include immutable hash and size metadata so envelopes stay small and replayable.

### 3.6 Encoding and Registry Contract

For typed projection support, stored payload bytes SHOULD use msgpack with stable numeric field tags.

Rules:
- Every persisted turn MUST include `type_id` and `type_version`.
- Forge-owned turn schemas SHOULD use a dedicated namespace (`forge.agent.*`, `forge.attractor.*`, `forge.link.*`, `forge.shared.*`).
- Registry bundles SHOULD be published before or alongside first writes for new schema versions.
- Unknown fields/tags MUST be forward-compatible and MUST NOT break readers.
- Production writers SHOULD encode payloads directly as msgpack with numeric tags (not JSON-to-msgpack transcode) for deterministic hashing.
- Cross-check references: `spec/cxdb/type-registry.md`, `spec/cxdb/http-api.md` (Registry)

### 3.7 Branch Context Policy

To preserve clear causality and avoid interleaving ambiguity:

- Agent: one context per session thread; each subagent gets its own forked context.
- Attractor: each parallel fan-out branch SHOULD run in its own forked context derived from pre-branch routing turn.
- Fan-in SHOULD emit explicit merge/fan-in turns that reference source branch context IDs and terminal turn IDs.

---

## 4. Integration Contracts

### 4.1 Turn Store Interface

Implementations SHOULD expose a minimal append/fork/read store interface:

```
TYPE ContextId = String            // Opaque in Forge, u64-backed in CXDB
TYPE TurnId = String               // Opaque in Forge, u64-backed in CXDB
TYPE BlobHash = String             // Lowercase hex BLAKE3-256

RECORD AppendTurnRequest:
    context_id       : ContextId
    parent_turn_id   : TurnId | None
    type_id          : String
    type_version     : Integer
    payload          : Bytes
    idempotency_key  : String

INTERFACE TurnStore:
    FUNCTION create_context(base_turn_id: TurnId | None) -> StoreContext
    FUNCTION append_turn(request: AppendTurnRequest) -> StoredTurn
    FUNCTION fork_context(from_turn_id: TurnId) -> StoreContext
    FUNCTION get_head(context_id: ContextId) -> StoredTurnRef
    FUNCTION list_turns(context_id: ContextId, before_turn_id: TurnId | None, limit: Integer) -> List<StoredTurn>
```

Optional extension methods:

```
INTERFACE TypedTurnStore EXTENDS TurnStore:
    FUNCTION publish_registry_bundle(bundle_id: String, bundle_json: Bytes) -> Void
    FUNCTION get_registry_bundle(bundle_id: String) -> Bytes | None

INTERFACE ArtifactStore:
    FUNCTION put_blob(raw_bytes: Bytes) -> BlobHash
    FUNCTION get_blob(content_hash: BlobHash) -> Bytes | None
    FUNCTION attach_fs(turn_id: TurnId, fs_root_hash: BlobHash) -> Void
```

### 4.2 Optional Coordination Interface (Distributed Runtime)

When running nodes across multiple processes/machines, an optional coordination interface MAY be implemented:

```
INTERFACE RunCoordinator:
    FUNCTION claim_node(run_id: String, node_id: String, worker_id: String, lease_ms: Integer) -> ClaimResult
    FUNCTION renew_lease(run_id: String, node_id: String, worker_id: String, lease_ms: Integer) -> LeaseResult
    FUNCTION release_node(run_id: String, node_id: String, worker_id: String, status: String) -> Void
```

Rules:
- Coordination APIs are optional and MUST NOT be required for single-process deterministic runtime.
- Coordination claims MUST be idempotent and lease-based.

### 4.3 Agent Hook Points

`forge-agent` SHOULD append store turns at:
- session start/end,
- input acceptance,
- assistant completion,
- tool call start/end (including truncation metadata),
- steering injection,
- subagent spawn/close linkage,
- checkpoint snapshot creation (when used).

### 4.4 Attractor Hook Points

Attractor runtime SHOULD append store turns at:
- pipeline start/finalization,
- every stage start/end/failure/retry,
- edge selection decision,
- human question/answer lifecycle,
- checkpoint save,
- stage-to-agent linkage creation.

### 4.5 Failure Handling Modes

Runtime config SHOULD support:
- `off`: no store writes.
- `best_effort`: write failures become warning events; runtime continues.
- `required`: write failures are terminal for that run.

Recommended defaults:
- agent: `best_effort`.
- attractor: `best_effort` initially; `required` for strict audit deployments.

### 4.6 Idempotency Keys

Integration metadata SHOULD include deterministic idempotency keys, for example:
- `run_id + node_id + stage_attempt_id + event_kind + sequence_no` (attractor),
- `session_id + local_turn_index + event_kind` (agent).

These keys are used to suppress duplicate writes during retries/reconnects.
CXDB v1 deduplicates idempotency keys per context with a 24-hour TTL; strict longer-horizon dedup SHOULD be handled by integration policy when needed.

### 4.7 CXDB Operation Mapping Contract

`forge-turnstore-cxdb` SHOULD map operations as follows:

- `create_context(base_turn_id)` -> binary `CTX_CREATE` (`base_turn_id=0` for empty).
- `fork_context(from_turn_id)` -> binary `CTX_FORK`.
- `append_turn` -> binary `APPEND_TURN` (client computes BLAKE3 over uncompressed payload, sets encoding/compression/idempotency).
- `get_head` -> binary `GET_HEAD`.
- `list_turns` newest-window, raw bytes -> binary `GET_LAST`.
- `list_turns` with cursor paging (`before_turn_id`) or typed projection -> HTTP `GET /v1/contexts/:id/turns`.
- `publish_registry_bundle`/`get_registry_bundle` -> HTTP `/v1/registry/*`.
- `put_blob`/`get_blob`/`attach_fs` -> binary `PUT_BLOB`/`GET_BLOB`/`ATTACH_FS`.

Adapters SHOULD keep Forge IDs opaque and perform u64/string conversion only at the CXDB boundary.
Cross-check references: `spec/cxdb/protocol.md`, `spec/cxdb/http-api.md`, `spec/cxdb/architecture.md`

---

## 5. Runtime Semantics

### 5.1 Source of Truth by Phase

- Pre-CXDB phases: in-memory/filesystem state remains authoritative; turnstore implementations are interchangeable for tests and early integration.
- CXDB mirror phase: runtime state remains authoritative locally, CXDB is mirrored journal.
- CXDB-authoritative phase (opt-in): resume/checkpoint and branch introspection may restore from CXDB directly.

### 5.2 Filesystem and In-memory Requirements

Even when CXDB is primary, implementations SHOULD retain:
- filesystem execution workspace for tool operations and deterministic local harnessing,
- in-memory turnstore implementation for unit tests and deterministic integration tests,
- optional filesystem-backed turnstore implementation for parity and offline debugging.

Stage artifact writes (`prompt.md`, `response.md`, `status.json`) SHOULD be runtime-configurable:
- `required` (always write),
- `mirror` (write when configured),
- `off` (for pure DB-heavy modes).

### 5.3 Ordering and Idempotency

- Store writes MUST preserve causal order per context.
- Retries on append MUST be idempotent at integration layer.
- Cross-context causal links (fork/fan-in/stage-to-agent) MUST be explicit in payload metadata.

### 5.4 Branching

- Agent subagents SHOULD fork from the parent turn that triggered spawn.
- Attractor retry loops and parallel fan-out branches SHOULD fork from pre-branch routing turn.
- Merge/fan-in turns SHOULD reference all source contexts that contributed.

### 5.5 Privacy and Retention

Implementations MUST allow:
- payload redaction for secrets,
- configurable retention/TTL policy,
- disabling persistence for sensitive projects.

### 5.6 Security Posture

Given CXDB v1 assumptions:
- Binary protocol endpoints MUST be treated as trusted-network-only surfaces.
- Production deployments SHOULD place HTTP endpoints behind authenticated gateway/proxy.
- Forge integrations MUST support transport security (TLS or equivalent network controls).
- Secret-bearing fields SHOULD be redacted before persistence when policy requires.

### 5.7 Renderer Boundary

Renderer loading/execution is a host/UI concern, not a core runtime concern:
- Core engine libraries MUST only persist typed turns and emit events.
- Host/web surfaces MAY consume typed projections and map `type_id` to renderers.
- Remote renderer code loading MUST be origin-restricted by host configuration.

---

## 6. Rollout Plan

### Phase A: Pre-runtime storage foundations (before Attractor runtime milestones)
- Add `forge-turnstore` interfaces and shared envelope/correlation types.
- Add optional artifact/blob extension interface(s) so large payloads are not forced into turn bodies.
- Add in-memory implementation for deterministic tests.
- Add optional filesystem implementation for local parity/offline workflows.
- Integrate `forge-agent` with optional store binding and deterministic idempotency metadata.

### Phase B: Attractor runtime built on storage abstractions
- Build Attractor runtime modules to depend on storage interfaces from day one.
- Persist stage/run/checkpoint records through abstraction (no CXDB requirement yet).
- Persist DOT source + normalized graph snapshot through abstraction.

### Phase C: CXDB adapter integration (allowed after P31)
- Implement `forge-turnstore-cxdb` adapter.
- Implement binary write path + HTTP projection/read path split per Section 4.7.
- Dual-write runtime events/turns in `best_effort` mode.
- Add projection browsing for stage<->agent drill-down.

### Phase D: CXDB-authoritative and distributed coordination (opt-in)
- Restore checkpoint/session state from CXDB when enabled.
- Add conformance tests for replay parity (filesystem vs CXDB-backed).
- Add optional lease-based coordination for multi-process execution.

---

## 7. Definition of Done

### 7.1 Architecture
- [ ] Core crates depend on storage interfaces, not CXDB concrete types.
- [ ] `forge-turnstore` abstraction crate exists with deterministic in-memory implementation.
- [ ] `forge-llm` has no CXDB coupling.
- [ ] CXDB adapter is isolated in a dedicated crate/module.

### 7.2 Agent Integration
- [ ] Session lifecycle and turn types can be persisted via store interface.
- [ ] Subagent spawn creates fork/link records with context lineage metadata.
- [ ] Store failures follow configured mode (`off`, `best_effort`, `required`).
- [ ] Agent payloads include deterministic idempotency and correlation metadata.

### 7.3 Attractor Integration
- [ ] Stage lifecycle, routing decisions, and checkpoint metadata are persisted.
- [ ] Stage-to-agent linkage records are emitted and queryable.
- [ ] DOT source and normalized graph snapshot are persisted per run.
- [ ] Parallel fan-out/fan-in branch context policy is implemented and tested.

### 7.4 CXDB Integration (Post-P31 allowed)
- [ ] `forge-turnstore-cxdb` append/fork/read path is implemented.
- [ ] Binary-vs-HTTP protocol usage follows Section 2.4 defaults.
- [ ] Adapter mapping follows Section 4.7 including blob/artifact operations when enabled.
- [ ] Replay/resume parity tests pass for CXDB-backed runs (when enabled).
- [ ] Host drill-down view can navigate run -> stage -> agent -> tool-turn timeline.

### 7.5 Operational Concerns
- [ ] Redaction/retention controls are implemented.
- [ ] Integration can be disabled with zero behavior change to core loops.
- [ ] Event ordering and append idempotency are verified in tests.
- [ ] Registry bundle publication/versioning is part of rollout and CI checks.
- [ ] Renderer loading remains host-scoped with origin restrictions.
