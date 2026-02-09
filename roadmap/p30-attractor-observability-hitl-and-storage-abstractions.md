# P30: Attractor Observability, HITL Surfaces, and Storage Abstractions (Spec 03 §6, §9)

**Status**
- Planned (2026-02-09)

**Goal**
Implement the host-facing integration surfaces: event stream, interviewer implementations, optional HTTP mode, and storage abstractions with filesystem-authoritative persistence.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 6, 9.5, 9.6, 9.7, 11.8, 11.11)
- Storage direction: `spec/04-cxdb-integration-spec.md` (Sections 2, 4, 5, 6 Phase 1)

**Context**
- Runtime core exists from P28/P29.
- We need clean host integration boundaries for CLI/TUI/Web without coupling runtime logic to one frontend.
- Filesystem artifacts remain authoritative in this phase; turn-store writes are optional mirror behavior.

## Scope
- Implement typed runtime events and streaming APIs.
- Implement interviewer interfaces and concrete implementations.
- Implement optional HTTP server surface for pipeline control and human answers.
- Introduce storage interfaces with filesystem implementation as default authority.
- Add optional best-effort turn mirroring hooks (interface only, no CXDB implementation yet).

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

### [ ] G3. Storage interfaces + filesystem authoritative implementation
- Work:
  - Introduce runtime storage interfaces:
    - run metadata + checkpoint persistence
    - stage artifact/status writes
    - optional event/turn mirror sink
  - Implement filesystem-backed store used as authoritative state.
  - Keep runtime behavior identical when mirror sink is disabled.
- Files:
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/storage/fs.rs`
  - `crates/forge-attractor/src/storage/memory.rs`
- DoD:
  - Core runtime depends on interfaces; filesystem store is default and complete.

### [ ] G4. Optional HTTP server mode (feature-gated)
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

### [ ] G6. Integration tests for event/HITL/server/storage behavior
- Work:
  - Add tests for event ordering and payload shape.
  - Add tests for queue/callback interviewer flows.
  - Add tests for HTTP lifecycle endpoints and answer submission.
  - Add tests confirming no behavior regression when mirror sink is off.
- Files:
  - `crates/forge-attractor/tests/events.rs`
  - `crates/forge-attractor/tests/hitl.rs`
  - `crates/forge-attractor/tests/http.rs`
  - `crates/forge-attractor/tests/storage.rs`
- DoD:
  - Host integration surfaces are stable and deterministic in test runs.

## Deliverables
- Event stream contract for UI/logging integration.
- Full interviewer interface and implementations.
- Feature-gated HTTP server mode.
- Storage abstraction with filesystem-authoritative backend and optional mirror sink hooks.

## Execution order
1. G1 event model
2. G2 interviewer implementations
3. G3 storage interfaces + FS store
4. G4 HTTP server feature
5. G5 tool hook bridge
6. G6 integration tests

## Exit criteria for this file
- Runtime is fully operable headlessly and via host surfaces.
- Filesystem remains the authoritative persistence path.
- Storage abstraction is in place for future CXDB adapter without runtime rewrites.

