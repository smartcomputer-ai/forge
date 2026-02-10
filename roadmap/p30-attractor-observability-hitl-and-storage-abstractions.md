# P30: Attractor Observability, HITL Surfaces, and In-Process CLI Host (Spec 03 §6, §9)

**Status**
- Planned (2026-02-09)
- Scope updated to CLI-first in-process host (2026-02-10)

**Goal**
Implement host-facing integration surfaces for a CLI-first in-process host: typed event stream, interviewer implementations, storage-backed query APIs, and hook observability, while consuming storage abstractions introduced earlier.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 6, 9.6, 9.7, 11.8, 11.11; 9.5 intentionally deferred)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.4, 4.4, 5.7)
- Prerequisite: `roadmap/p27.1-turnstore-foundation-and-agent-persistence.md`

**Context**
- Runtime core exists from P28/P29.
- First host target is a CLI that embeds the runtime in-process (no daemon/server dependency).
- We need clean host integration boundaries without coupling runtime logic to transport/UI concerns.
- Storage interfaces exist already; this phase consumes them for query and observability surfaces.

## Scope
- Implement typed runtime events and streaming APIs.
- Implement interviewer interfaces and concrete implementations.
- Implement an in-process CLI host surface for run/resume/inspection and human-gate interaction.
- Expose read/query surfaces over storage-backed runtime state for host introspection.
- Integrate tool hook observability bridge in codergen flows.

## Out of Scope
- HTTP server mode and SSE transport surface (`spec/03` Section 9.5) in this phase.
- Out-of-process runtime hosting (daemon/service) and remote control protocol.
- Web/TUI host implementation.
- CXDB adapter crate and production deployment hardening for CXDB transport.
- Remote renderer loading/projection UI implementation.
- Non-headless host UX polish.

## Priority 0 (Must-have)

### [x] G1. Typed event model + observer/stream APIs
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
- Completed:
  - Added typed runtime event model in `crates/forge-attractor/src/events.rs` covering pipeline, stage, parallel, interview, and checkpoint lifecycles.
  - Added observer callback + async stream sinks (`RuntimeEventSink`, `runtime_event_channel`) and surfaced configuration through `RunConfig.events`.
  - Integrated event emission into `PipelineRunner` for run lifecycle, stage lifecycle/retry, parallel node branch summaries, interview lifecycle, and checkpoint save notifications.
  - Added coverage for event sink fanout and runner event sequencing.

### [x] G2. Interviewer abstractions and implementations
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
- Completed:
  - Added `crates/forge-attractor/src/interviewer.rs` with shared `Interviewer` contract and built-in implementations:
    - `AutoApproveInterviewer`
    - `ConsoleInterviewer`
    - `CallbackInterviewer`
    - `QueueInterviewer`
    - `RecordingInterviewer`
  - Updated `wait.human` to consume shared interviewer types and added support for node-configured timeout (`human.timeout_seconds`) and default choice fallback (`human.default_choice`/`human_default_choice`).
  - Added deterministic timeout/default-choice tests in `crates/forge-attractor/src/handlers/wait_human.rs` and interviewer implementation tests in `crates/forge-attractor/src/interviewer.rs`.

### [x] G3. In-process CLI host surface
- Work:
  - Add CLI host entrypoint that directly invokes Attractor runtime/library APIs.
  - Support initial command surface for:
    - run pipeline from DOT source/file
    - resume from checkpoint
    - stream/print typed runtime events
    - inspect current checkpoint/context and run status
    - provide human answers through interviewer-backed flows
  - Keep runtime modules transport-agnostic; CLI adapter owns presentation/IO.
- Files:
  - `crates/forge-cli/src/main.rs`
- DoD:
  - Runtime is operable end-to-end through an embedded CLI process with no external service dependency.
- Completed:
  - Added dedicated host crate `crates/forge-cli/` and wired it into workspace membership.
  - Implemented `forge-cli run` to execute pipelines from `--dot-file` or `--dot-source`, with optional typed runtime event streaming (`--event-json`, `--no-stream-events`).
  - Implemented `forge-cli resume` to continue from `--checkpoint` using the same in-process runtime entrypoints.
  - Implemented `forge-cli inspect-checkpoint` for checkpoint/context/run-status inspection (human-readable and JSON output modes).
  - Integrated interviewer-backed human-gate behavior selection via CLI (`--interviewer auto|console|queue` + `--human-answer` for queue mode) while keeping runtime transport-agnostic.

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

### [ ] G6. Integration tests for event/HITL/CLI/query behavior
- Work:
  - Add tests for event ordering and payload shape.
  - Add tests for queue/callback interviewer flows.
  - Add tests for CLI-hosted run/resume and interactive answer flows.
  - Add tests for storage-backed query parity across in-memory and filesystem backends.
- Files:
  - `crates/forge-attractor/tests/events.rs`
  - `crates/forge-attractor/tests/hitl.rs`
  - `crates/forge-attractor/tests/cli_host.rs`
  - `crates/forge-attractor/tests/queries.rs`
- DoD:
  - Host integration surfaces are stable and deterministic in test runs.

## Deliverables
- Event stream contract for UI/logging integration.
- Full interviewer interface and implementations.
- In-process CLI host command surface.
- Backend-agnostic host query APIs over storage-backed state.

## Execution order
1. G1 event model
2. G2 interviewer implementations
3. G3 in-process CLI host surface
4. G4 query surfaces
5. G5 tool hook bridge
6. G6 integration tests

## Exit criteria for this file
- Runtime is fully operable headlessly and through an in-process CLI host.
- Host surfaces are backend-agnostic and storage-aware.
- HTTP/out-of-process host mode is explicitly deferred to a later roadmap phase.
- Observability APIs are ready for post-P31 CXDB projection adoption.
