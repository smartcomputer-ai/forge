# P26: Agent Live Integration Smoke Tests (Spec 02 §9.13)

**Status**
- In progress (2026-02-09)

**Goal**
Add a small, default-ignored live integration suite for `forge-agent` that validates real provider behavior without replacing deterministic mocked conformance tests.

**Source**
- Spec of record: `spec/02-coding-agent-loop-spec.md` (Section 9.13)
- Tracking matrix: `roadmap/p24-dod-matrix.md` (9.13 real-key run notes)
- Existing live-test pattern: `crates/forge-llm/tests/openai_live.rs`, `crates/forge-llm/tests/anthropic_live.rs`

**Context**
- `crates/forge-agent/tests/` already provides broad deterministic conformance via scripted adapters.
- Spec 9.13 explicitly calls for real-key smoke coverage.
- We need low-flake, low-cost tests that assert runtime side effects and event contracts, not brittle assistant wording.

## Scope
- Add live tests for OpenAI and Anthropic profiles.
- Keep tests `#[ignore]` + env-gated.
- Reuse the same operational style as `forge-llm` live tests (dotenv fallback, retries, bounded timeouts).
- Capture real-key execution notes and close the outstanding DoD matrix item.

## Out of Scope
- Replacing mocked conformance tests with live tests.
- Exhaustive cross-provider parity under live keys.
- Hard CI gating on external-provider availability.
- Gemini live coverage in this phase (can be added as follow-up).

## Priority 0 (Must-have)

### [ ] G1. Add shared live-test harness for `forge-agent`
- Work:
  - Add helper utilities for:
    - env or `.env` variable resolution
    - live test enablement flags
    - retry/backoff wrappers for transient provider failures
    - fixed event wait timeouts
    - temp workspace/session bootstrap
  - Keep helper API minimal to avoid over-abstraction.
- Candidate files:
  - `crates/forge-agent/tests/support/live.rs` (new)
  - `crates/forge-agent/tests/support/mod.rs` (wire live helpers)
- DoD:
  - OpenAI and Anthropic live tests share one helper path for env/retry/bootstrap.

### [ ] G2. OpenAI live smoke tests (default ignored)
- Work:
  - Add `crates/forge-agent/tests/openai_live.rs`.
  - Gate with `RUN_LIVE_OPENAI_TESTS=1` and `OPENAI_API_KEY` (env or `.env`).
  - Optional model override via `OPENAI_LIVE_MODEL` (default: conservative low-cost model).
- Scenarios:
  - file create + edit smoke (assert filesystem side effects)
  - tool-output truncation smoke (assert warning marker + full output in `TOOL_CALL_END`)
  - shell timeout smoke (assert timeout handling path)
  - submit-options smoke (assert provider/model/reasoning overrides applied end-to-end)
- DoD:
  - `cargo test -p forge-agent --test openai_live -- --ignored` passes with valid key.

### [ ] G3. Anthropic live smoke tests (default ignored)
- Work:
  - Add `crates/forge-agent/tests/anthropic_live.rs`.
  - Gate with `RUN_LIVE_ANTHROPIC_TESTS=1` and `ANTHROPIC_API_KEY` (env or `.env`).
  - Optional model override via `ANTHROPIC_LIVE_MODEL`.
- Scenarios:
  - same minimal smoke set as OpenAI, using Anthropic profile/tool conventions
- DoD:
  - `cargo test -p forge-agent --test anthropic_live -- --ignored` passes with valid key.

### [ ] G4. Real-key run notes + DoD matrix closure
- Work:
  - Record one real-key run summary (date, provider, model, command, pass/fail notes, known flakes).
  - Update `roadmap/p24-dod-matrix.md`:
    - mark 9.13 "Real-key run notes captured" as complete once evidence is documented.
  - Add crate-level run instructions for live tests in `crates/forge-agent/README.md`.
- DoD:
  - 9.13 real-key item is checked with traceable run notes.

## Priority 1 (Follow-up)

## Test design constraints
- Assertions should be on:
  - filesystem/tool side effects
  - session state transitions
  - event payload invariants
- Avoid assertions on:
  - exact assistant wording
  - exact tool-call count/order unless contract-critical
- Keep each test short and single-purpose to limit token cost and flake surface.

## Deliverables
- New live integration files:
  - `crates/forge-agent/tests/openai_live.rs`
  - `crates/forge-agent/tests/anthropic_live.rs`
  - (optional) `crates/forge-agent/tests/gemini_live.rs`
  - `crates/forge-agent/tests/support/live.rs`
- Docs/roadmap updates:
  - `crates/forge-agent/README.md` live-test section
  - `roadmap/p24-dod-matrix.md` 9.13 real-key checkmark + notes

## Execution order
1. G1 shared live harness
2. G2 OpenAI live suite
3. G3 Anthropic live suite
4. G4 run notes + DoD matrix update

## Exit criteria for this file
- OpenAI + Anthropic live smoke tests exist, are `#[ignore]`, and pass with valid credentials.
- Live-test commands and env requirements are documented.
- `roadmap/p24-dod-matrix.md` 9.13 real-key item is closed with dated run notes.
