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
- attractor stage transitions, outcomes, and human-gate decisions,
- checkpoint metadata for replay and resume.

### 1.3 Non-goals

- Replacing `forge-llm` request/response contracts.
- Coupling core loop correctness to CXDB availability by default.
- Removing filesystem artifacts from `03-attractor-spec.md` in the first integration phase.

---

## 2. Layering and Boundaries

### 2.1 Layer Placement

CXDB integration belongs above the LLM transport layer and below hosts/UI:

- `forge-llm`: no direct CXDB dependency.
- `forge-agent`: optional turn persistence adapter.
- `forge-attractor` (or equivalent pipeline crate): optional run/turn persistence adapter.
- Host (CLI/HTTP/TUI/Web): reads events from engine and may read projections from CXDB.

### 2.2 Architecture Rule

Core runtimes MUST depend on storage interfaces, not on CXDB-specific SDK types.

CXDB integration MUST be implemented as an adapter module/crate, e.g.:
- `forge-turnstore` (traits + shared types),
- `forge-turnstore-cxdb` (CXDB implementation).

### 2.3 Why this Layer

This placement preserves:
- deterministic core execution when storage is disabled,
- portability to other backing stores,
- testability through in-memory fake stores.

### 2.4 CXDB Protocol Usage

Integrations SHOULD use CXDB protocols by responsibility:

- Write-heavy runtime path (append/fork/head updates): prefer CXDB binary protocol (`:9009`) for throughput.
- Read/projection path (UI/tooling/timeline browsing): prefer CXDB HTTP API (`:9010`).
- HTTP-only write mode is acceptable for bootstrap/testing, but SHOULD NOT be the default for production append paths.

---

## 3. Data Model Mapping

### 3.1 Mapping Principles

- One CXDB context represents one logical execution thread.
- Every emitted runtime event/turn can be represented as an immutable CXDB turn.
- Parent-child relationships mirror causality and branch points.

### 3.2 Agent Mapping (`02-coding-agent-loop-spec.md`)

Recommended mapping:
- Agent session root -> new CXDB context.
- `UserTurn`, `AssistantTurn`, `ToolResultsTurn`, `SteeringTurn`, `SystemTurn` -> CXDB turns.
- Subagent spawn -> forked CXDB context from parent turn where spawn occurred.

Recommended `type_id` namespace:
- `forge.agent.user_turn`
- `forge.agent.assistant_turn`
- `forge.agent.tool_results_turn`
- `forge.agent.steering_turn`
- `forge.agent.system_turn`
- `forge.agent.event`

### 3.3 Attractor Mapping (`03-attractor-spec.md`)

Recommended mapping:
- Pipeline run root -> new CXDB context.
- Stage lifecycle events -> turns (`StageStarted`, `StageCompleted`, `StageFailed`, etc.).
- Human interaction (`InterviewStarted`, `InterviewCompleted`) -> turns.
- Checkpoint save -> turn containing checkpoint pointer/hash + minimal state summary.
- Retry/failure routing -> normal turns with explicit edge-selection metadata.

Recommended `type_id` namespace:
- `forge.attractor.stage_event`
- `forge.attractor.interview_event`
- `forge.attractor.checkpoint_event`
- `forge.attractor.route_decision`
- `forge.attractor.run_event`

### 3.4 Payload Requirements

Stored payloads SHOULD be stable, versioned envelopes:

```
RECORD StoredTurnEnvelope:
    schema_version : Integer
    run_id         : String | None
    session_id     : String | None
    node_id        : String | None
    event_kind     : String
    timestamp      : Timestamp
    payload        : Object
```

Rules:
- `schema_version` MUST be present for migration safety.
- `payload` MUST avoid non-deterministic fields unless explicitly marked diagnostic.
- Large blobs SHOULD be stored as artifacts with references in the envelope.

### 3.5 Encoding and Registry Contract

For typed projection support, stored payload bytes SHOULD use msgpack with stable numeric field tags.

Rules:
- Every persisted turn MUST include `type_id` and `type_version`.
- Forge-owned turn schemas SHOULD use a dedicated namespace (`forge.agent.*`, `forge.attractor.*`, `forge.shared.*`).
- Registry bundles SHOULD be published before or alongside first writes for new schema versions.
- Unknown fields/tags MUST be forward-compatible and MUST NOT break readers.

### 3.6 Branch Context Policy

To preserve clear causality and avoid interleaving ambiguity:

- Agent: one CXDB context per session thread; each subagent gets its own forked context.
- Attractor: each parallel fan-out branch SHOULD run in its own forked context derived from the pre-branch routing turn.
- Fan-in SHOULD emit explicit merge/fan-in turns that reference source branch context IDs and terminal turn IDs.

---

## 4. Integration Contracts

### 4.1 Storage Interface

Implementations SHOULD expose a minimal store interface:

```
INTERFACE TurnStore:
    FUNCTION create_context(parent_turn_id: String | None) -> StoreContext
    FUNCTION append_turn(context_id: String, type_id: String, type_version: Integer, payload: Bytes) -> StoredTurn
    FUNCTION fork_context(from_context_id: String, from_turn_id: String) -> StoreContext
    FUNCTION get_head(context_id: String) -> StoredTurnRef
    FUNCTION list_turns(context_id: String, before_turn_id: String | None, limit: Integer) -> List<StoredTurn>
```

Optional extension methods:

```
INTERFACE TypedTurnStore EXTENDS TurnStore:
    FUNCTION publish_registry_bundle(bundle_id: String, bundle_json: Bytes) -> Void
    FUNCTION get_registry_bundle(bundle_id: String) -> Bytes | None
```

### 4.2 Agent Hook Points

`forge-agent` SHOULD append store turns at:
- input acceptance,
- assistant completion,
- tool call start/end (including truncation metadata),
- steering injection,
- session start/end.

### 4.3 Attractor Hook Points

Attractor engine SHOULD append store turns at:
- pipeline start/finalization,
- every stage start/end/failure/retry,
- checkpoint save,
- edge selection decision,
- human question/answer lifecycle.

### 4.4 Failure Handling Modes

Runtime config SHOULD support:
- `off`: no store writes.
- `best_effort`: write failures become warning events; runtime continues.
- `required`: write failures are terminal for that run.

Recommended default:
- agent: `best_effort`
- attractor: `best_effort` in early rollout, optional `required` for strict audit deployments.

### 4.5 Idempotent Append Key

Integration payload metadata SHOULD include a deterministic idempotency key, for example:
- `run_id + stage_id + event_kind + sequence_no` (attractor),
- `session_id + turn_local_index + event_kind` (agent).

This key is used to suppress duplicate writes during retries/reconnects.

---

## 5. Runtime Semantics

### 5.1 Source of Truth by Phase

- Initial phase: existing in-memory/filesystem state remains authoritative; CXDB is a mirrored journal.
- Later phase (opt-in): resume/checkpoint may use CXDB as authoritative backing store.

### 5.2 Ordering and Idempotency

- Store writes MUST preserve causal order per context.
- Retries on append MUST be idempotent at integration layer (e.g., deterministic dedup keys in payload metadata).

### 5.3 Branching

- Agent subagents SHOULD fork from the parent turn that triggered spawn.
- Attractor retry loops and parallel fan-out branches SHOULD fork from the pre-branch routing turn.

### 5.4 Privacy and Retention

Implementations MUST allow:
- payload redaction for secrets,
- configurable retention/TTL policy,
- disabling persistence for sensitive projects.

### 5.5 Security Posture

Given CXDB v1 assumptions:
- Binary protocol endpoints MUST be treated as trusted-network-only surfaces.
- Production deployments SHOULD place HTTP endpoints behind an authenticated gateway/proxy.
- Forge integrations MUST support transport security (TLS or equivalent network controls).
- Secret-bearing fields SHOULD be redacted before persistence when policy requires.

### 5.6 Renderer Boundary

Renderer loading/execution is a host/UI concern, not a core runtime concern:
- Core engine libraries MUST only persist typed turns and emit events.
- Host/web surfaces MAY consume CXDB typed projections and map `type_id` to renderers.
- Remote renderer code loading MUST be origin-restricted (CSP/allowlist) by host configuration.

---

## 6. Rollout Plan

### Phase 1: Mirror-only integration
- Add storage interfaces and in-memory test implementation.
- Add CXDB adapter.
- Add initial Forge registry bundle(s) for agent/attractor type IDs and versions.
- Dual-write runtime events/turns in `best_effort` mode.
- Keep current checkpoints/log files authoritative.

### Phase 2: Resume-aware integration
- Persist checkpoint snapshots/pointers in CXDB.
- Support restoring run/session state from CXDB.
- Add conformance tests for replay parity (filesystem vs CXDB-backed).

### Phase 3: Host-level integration
- Expose run/session browsing via host surfaces (CLI/HTTP/TUI/Web).
- Use typed projections for timeline rendering and branch diffing.

---

## 7. Definition of Done

### 7.1 Architecture
- [ ] Core crates depend on a storage interface, not CXDB concrete types
- [ ] CXDB adapter is isolated in a dedicated module/crate
- [ ] `forge-llm` has no CXDB coupling
- [ ] Binary-vs-HTTP protocol usage follows Section 2.4 defaults

### 7.2 Agent Integration
- [ ] Session lifecycle and turn types are persisted through the store interface
- [ ] Subagent spawn creates or links forked contexts
- [ ] Store failures follow configured mode (`off`, `best_effort`, `required`)
- [ ] Agent turn payloads include deterministic idempotency metadata

### 7.3 Attractor Integration
- [ ] Stage lifecycle and routing decisions are persisted
- [ ] Checkpoint metadata is persisted per node completion
- [ ] Replay/resume parity tests pass for CXDB-backed runs (when enabled)
- [ ] Parallel fan-out/fan-in context policy is implemented and tested

### 7.4 Operational Concerns
- [ ] Redaction/retention controls are implemented
- [ ] Integration can be disabled with zero behavior change to core loops
- [ ] Event ordering and append idempotency are verified in tests
- [ ] Registry bundle publication/versioning is part of rollout and CI checks
- [ ] Renderer loading remains host-scoped with origin restrictions
