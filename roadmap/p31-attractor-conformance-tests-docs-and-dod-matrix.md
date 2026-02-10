# P31: Attractor Conformance Tests, Docs, and DoD Matrix Closure (Spec 03 §11)

**Status**
- Planned (2026-02-09)
- In progress (2026-02-10)

**Goal**
Close the Attractor implementation loop with a comprehensive DoD matrix, deterministic conformance coverage, integration smoke tests, and documentation updates.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Section 11 and Appendix references)
- Storage extension: `spec/04-cxdb-integration-spec.md` (pre-CXDB phases)
- Workspace docs baseline: `README.md`, `crates/forge-agent/README.md`

**Context**
- P27.1 and P28-P30 deliver implementation slices.
- This phase enforces completeness against spec/03 Section 11 and provides clear audit evidence of conformance.
- CXDB adapter work remains intentionally deferred until after this closure milestone.

## Scope
- Create and maintain a Section 11 DoD matrix roadmap tracker.
- Add deterministic conformance test suites mapped to each DoD section.
- Add an optional default-ignored live smoke path for codergen backend integration.
- Validate deterministic parity across supported local storage backends (in-memory/filesystem).
- Update workspace/crate docs for Attractor usage and test execution.

## Out of Scope
- New runtime features not required by spec/03.
- CXDB adapter implementation and production rollout.
- UI rendering layer beyond runtime API contracts.

## Priority 0 (Must-have)

### [x] G1. Add Attractor DoD matrix tracker
- Work:
  - Create roadmap matrix file mirroring spec/03 Section 11 checklist structure.
  - Map each matrix row to concrete files/tests.
  - Update matrix continuously as milestones close.
- Files:
  - `roadmap/p31-dod-matrix.md` (new)
- DoD:
  - Every Section 11 item has an explicit check state and traceable implementation reference.
- Completed:
  - Added `roadmap/p31-dod-matrix.md` with item-by-item mapping for spec/03 Section 11.
  - Marked each checklist row with explicit `[x]` / `[ ]` state and test/file references.
  - Captured current known deltas (for example exact terminal-node-count semantics and deferred live smoke coverage).

### [x] G2. Deterministic conformance suite (parse -> validate -> execute -> resume)
- Work:
  - Build full matrix-aligned deterministic tests covering:
    - parsing/validation
    - execution/routing/retry/gates
    - handlers
    - state/checkpoint/resume
    - condition language
    - stylesheet/transforms
  - Use queue interviewer and mocked codergen backend for deterministic behavior.
  - Run suites with in-memory and filesystem storage backends.
- Files:
  - `crates/forge-attractor/tests/conformance_parsing.rs`
  - `crates/forge-attractor/tests/conformance_runtime.rs`
  - `crates/forge-attractor/tests/conformance_state.rs`
  - `crates/forge-attractor/tests/conformance_stylesheet.rs`
- DoD:
  - Section 11 cross-feature parity matrix is fully automated and green.
- Completed:
  - Added new conformance suites:
    - `crates/forge-attractor/tests/conformance_parsing.rs`
    - `crates/forge-attractor/tests/conformance_runtime.rs`
    - `crates/forge-attractor/tests/conformance_state.rs`
    - `crates/forge-attractor/tests/conformance_stylesheet.rs`
  - Confirmed deterministic coverage across parse/validate/runtime/state/stylesheet-transform domains, including interviewer + mocked codergen flows.
  - Executed suites with both in-memory and filesystem turnstore backends in runtime/state conformance tests.
  - Incorporated and credited existing coverage already added across P28-P30 test files (`execution_core.rs`, `state_and_resume.rs`, `conditions.rs`, `parallel.rs`, `stylesheet.rs`, `hitl.rs`, `events.rs`, `queries.rs`).

### [x] G3. End-to-end integration smoke test (spec-style scenario)
- Work:
  - Add spec-inspired pipeline smoke test:
    - `plan -> implement -> review -> done`
    - success/fail routing paths
    - goal-gate verification
    - checkpoint verification
  - Assert artifact outputs (`prompt.md`, `response.md`, `status.json`) and context continuity.
- Files:
  - `crates/forge-attractor/tests/integration_smoke.rs`
- DoD:
  - Spec Section 11.13 smoke scenario is reproducible in CI without live keys.
- Completed:
  - Added deterministic spec-style smoke test in `crates/forge-attractor/tests/integration_smoke.rs`:
    - `plan -> implement -> review -> done` flow
    - fail/success routing path coverage (`implement -> plan` retry route)
    - goal-gate stage completion assertion
    - checkpoint assertions
    - artifact assertions for `prompt.md`, `response.md`, `status.json`
    - context continuity assertions

### [x] G4. Documentation and workspace integration updates
- Work:
  - Update workspace `README.md` to include Attractor crate and status.
  - Add `crates/forge-attractor/README.md` with:
    - architecture/layering
    - run/test commands
    - current known gaps (if any)
  - Add roadmap links and usage notes for host integration.
- Files:
  - `README.md`
  - `crates/forge-attractor/README.md`
  - `AGENTS.md` (if architecture index changes materially)
- DoD:
  - Documentation reflects new crate capabilities and test strategy.
- Completed:
  - Updated workspace docs in `README.md` to include Attractor + CLI host status, usage, and test commands.
  - Added `crates/forge-attractor/README.md` documenting architecture/layering, run/test commands, known deferred gaps, and roadmap links.
  - `AGENTS.md` architecture index already reflected current crate layout; no material architecture-index change required.

## Priority 1 (Strongly recommended)

### [ ] G5. Default-ignored live codergen smoke tests (OpenAI/Anthropic via `forge-agent` backend)
- Work:
  - Add env-gated live smoke tests for Attractor runtime through `forge-agent` backend.
  - Keep tests minimal, low-token, and non-brittle.
  - Record dated run notes in roadmap matrix.
- Files:
  - `crates/forge-attractor/tests/openai_live.rs`
  - `crates/forge-attractor/tests/anthropic_live.rs`
  - `crates/forge-attractor/tests/support/live.rs`
- DoD:
  - Live tests are optional (`#[ignore]`), documented, and manually runnable.

## Deliverables
- Section 11 DoD matrix with traceable closure status.
- Deterministic conformance suite covering all implemented features.
- Integration smoke tests and artifact assertions.
- Updated docs for Attractor runtime adoption.

## Execution order
1. G1 DoD matrix file
2. G2 deterministic conformance suites
3. G3 integration smoke test
4. G4 documentation updates
5. G5 optional live smoke tests

## Exit criteria for this file
- Section 11 checklist is fully represented and statused in roadmap.
- Deterministic conformance tests pass in CI (both memory and filesystem storage backends).
- Project documentation accurately reflects Attractor implementation and usage.
