# P24: Coding Agent Loop DoD Matrix + Cross-Provider Conformance

**Status**
- In progress (2026-02-09)

**Goal**
Validate implementation completeness against Section 9 of `spec/02-coding-agent-loop-spec.md`.

**Scope**
- Create a DoD matrix document for all checklist categories (core loop, tools, env, prompts, subagents, errors, events).
- Add automated conformance tests across profiles (OpenAI, Anthropic, Gemini where available).
- Add integration smoke tests for file creation/editing, shell timeout, truncation, steering, and subagent flows.
- Capture provider gaps explicitly when blocked by upstream SDK/provider availability.

**Out of Scope**
- Production benchmarking and performance tuning.

**Deliverables**
- `roadmap/p24-dod-matrix.md` with actionable checklist references to tests/files.
- Integration test suite in `crates/forge-agent/tests/`.
- Pass/fail report and deferred-items log.

**Acceptance**
- DoD checklist items are all linked to concrete tests or implementation files.
- `cargo test -p forge-agent` passes for implemented profile coverage.
- Deferred items (if any) are explicitly marked with rationale and follow-up issue/milestone.

## Implemented
- Added cross-provider conformance integration tests in `crates/forge-agent/tests/conformance_matrix.rs`.
- Test harness mirrors the mocked-adapter pattern used in `crates/forge-llm/tests/cross_provider_conformance.rs`:
  - deterministic scripted `ProviderAdapter`
  - provider fixture loop across OpenAI/Anthropic/Gemini
  - assertions on history/tool results instead of fragile output text matching
- Covered flows:
  - simple file creation
  - read + edit
  - provider-native edit variant (`apply_patch` for OpenAI, `edit_file` for Anthropic/Gemini)
  - shell execution
  - shell timeout
  - grep + glob discovery
  - parallel tool-call round
  - subagent spawn + wait

## Validation
- `cargo test -p forge-agent` passes:
  - unit tests: 53 passed
  - integration tests: 3 passed (`conformance_matrix.rs`)

## Deferred
- Real-key smoke run notes for OpenAI/Anthropic/Gemini (not part of mocked deterministic CI tests).
- Remaining parity scenarios not yet covered in integration tests:
  - multi-file edit flow
  - large-output truncation behavior
  - mid-task steering behavior
  - reasoning effort change behavior
  - loop detection warning behavior
  - error recovery after tool failure
