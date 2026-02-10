# P30: Attractor Observability, HITL Surfaces, and Host APIs (Spec 03 §6, §9)

**Status**
- Planned (2026-02-09)

**Goal**
Implement host-facing integration surfaces: typed event stream, interviewer implementations, and optional HTTP mode, while consuming storage abstractions introduced earlier.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 6, 9.5, 9.6, 9.7, 11.8, 11.11)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.4, 4.4, 5.7)
- Prerequisite: `roadmap/p27.1-turnstore-foundation-and-agent-persistence.md`

**Context**
- Runtime core exists from P28/P29.
- We need clean host integration boundaries for CLI/TUI/Web without coupling runtime logic to one frontend.
- Storage interfaces exist already; this phase consumes them for query and observability surfaces.

## Scope
- Implement typed runtime events and streaming APIs.
- Implement interviewer interfaces and concrete implementations.
- Implement optional HTTP server surface for pipeline control and human answers.
- Expose read/query surfaces over storage-backed runtime state for host introspection.
- Integrate tool hook observability bridge in codergen flows.

## Out of Scope
- CXDB adapter crate and production deployment hardening for CXDB transport.
- Remote renderer loading/projection UI implementation.
- Non-headless host UX polish.

## Priority 0 (Must-have)

### [ ] G1. Typed event model + observer/stream APIs
- Work:
  - Implement event types for:
    - pipeline lifecycle
    - stage lifecycle
    - parallel branch lifecycle
    - interview lifecycle
    - checkpoint lifecycle
  - Provide callback and async stream consumption paths.
- Files:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Host applications can consume deterministic typed events in real time.

### [ ] G2. Interviewer abstractions and implementations
- Work:
  - Implement interviewer trait and core implementations:
    - auto-approve
    - console
    - callback
    - queue
    - recording wrapper
  - Implement timeout/default-choice behavior for `wait.human`.
- Files:
  - `crates/forge-attractor/src/interviewer.rs`
  - `crates/forge-attractor/src/handlers/wait_human.rs`
- DoD:
  - Human gate flows are testable deterministically and usable interactively.

### [ ] G3. Optional HTTP server mode (feature-gated)
- Work:
  - Add feature-gated HTTP API with endpoints for:
    - create/start pipeline
    - get status
    - stream events (SSE)
    - answer pending questions
    - get checkpoint/context
  - Keep server layer outside core execution modules.
- Files:
  - `crates/forge-attractor/src/server/mod.rs`
  - `crates/forge-attractor/src/server/routes.rs`
- DoD:
  - Host can drive runtime via HTTP in local/dev environments.

### [ ] G4. Storage-backed host query surfaces
- Work:
  - Expose host-facing query helpers for:
    - run metadata
    - stage timeline
    - checkpoint snapshot
    - stage-to-agent linkage metadata
  - Keep query contract backend-agnostic.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
- DoD:
  - Host query behavior is stable regardless of selected storage backend.

## Priority 1 (Strongly recommended)

### [ ] G5. Tool hook bridge integration (pre/post) for codergen paths
- Work:
  - Pass `tool_hooks.pre` / `tool_hooks.post` from graph/node attrs into codergen backend calls.
  - Use `forge-agent` hook extension seams added in P25.
  - Record hook outcomes in stage logs and events.
- Files:
  - `crates/forge-attractor/src/backends/forge_agent.rs`
  - `crates/forge-attractor/src/hooks.rs`
- DoD:
  - Tool hooks are observable and policy-enforceable without breaking core loop determinism.

### [ ] G6. Integration tests for event/HITL/server/query behavior
- Work:
  - Add tests for event ordering and payload shape.
  - Add tests for queue/callback interviewer flows.
  - Add tests for HTTP lifecycle endpoints and answer submission.
  - Add tests for storage-backed query parity across in-memory and filesystem backends.
- Files:
  - `crates/forge-attractor/tests/events.rs`
  - `crates/forge-attractor/tests/hitl.rs`
  - `crates/forge-attractor/tests/http.rs`
  - `crates/forge-attractor/tests/queries.rs`
- DoD:
  - Host integration surfaces are stable and deterministic in test runs.

## Deliverables
- Event stream contract for UI/logging integration.
- Full interviewer interface and implementations.
- Feature-gated HTTP server mode.
- Backend-agnostic host query APIs over storage-backed state.

## Execution order
1. G1 event model
2. G2 interviewer implementations
3. G3 HTTP server feature
4. G4 query surfaces
5. G5 tool hook bridge
6. G6 integration tests

## Exit criteria for this file
- Runtime is fully operable headlessly and via host surfaces.
- Host surfaces are backend-agnostic and storage-aware.
- Observability APIs are ready for post-P31 CXDB projection adoption.
