# P24: Coding Agent Loop DoD Matrix + Cross-Provider Conformance

**Status**
- Planned (2026-02-09)

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

