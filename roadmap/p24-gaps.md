# P24 Gap Closure Plan (Spec 02)

**Status**
- Open (2026-02-09)

**Goal**
- Close remaining implementation gaps between `spec/02-coding-agent-loop-spec.md` and `crates/forge-agent`.

**Source**
- Review baseline: `roadmap/p24-dod-matrix.md`
- Spec of record: `spec/02-coding-agent-loop-spec.md`

## Priority 0 (Behavioral correctness)

### G1. Wire `SessionConfig` command timeout policy into shell execution
- Spec refs: 2.2, 5.4, 9.4
- Current gap:
  - `SessionConfig.default_command_timeout_ms` / `max_command_timeout_ms` are defined but not used by shell dispatch.
  - Shell tool currently defers to environment internals only.
- Work:
  - Pass effective timeout policy from session config into shell execution path.
  - Enforce per-call `timeout_ms` clamped by session max.
  - Keep timeout error text unchanged unless spec requires adjustment.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/tools.rs`
  - `crates/forge-agent/src/execution.rs`
  - `crates/forge-agent/tests/conformance_matrix.rs`
- DoD:
  - Session config default/max alter runtime shell behavior in tests.
  - Per-call override works and is clamped by configured max.

### G2. Implement graceful abort/shutdown sequence
- Spec refs: 2.8, 9.11, Appendix B graceful shutdown
- Current gap:
  - Abort flag transitions state, but no explicit cancellation/termination orchestration for in-flight work at session layer.
- Work:
  - Add explicit shutdown path covering:
    - aborting in-flight loop work,
    - terminating active command processes,
    - closing subagents,
    - emitting final `SESSION_END` once.
  - Ensure no event loss on shutdown.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/execution.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Abort test verifies transition to `CLOSED`, subagents closed, and `SESSION_END` emitted.
  - Running shell command is terminated on abort.

### G3. Fix subagent semantics (`spawn_agent` async behavior + arg support)
- Spec refs: 7.2, 7.3, 9.9
- Current gap:
  - `spawn_agent` currently blocks until child completes.
  - `working_dir` and `model` args are accepted by schema but ignored.
- Work:
  - Return immediately after spawn with running status.
  - Honor `working_dir` scoping and `model` override.
  - Keep `wait` / `send_input` / `close_agent` consistent with new lifecycle.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/tools.rs`
  - `crates/forge-agent/tests/conformance_matrix.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - `spawn_agent` returns before child completion.
  - `wait` returns final output deterministically.
  - `working_dir` and `model` are observable in behavior/tests.

## Priority 1 (Spec parity)

### G4. Emit all declared event kinds at correct times
- Spec refs: 2.9, 9.10
- Current gap:
  - `ASSISTANT_TEXT_DELTA` and `TOOL_CALL_OUTPUT_DELTA` kinds exist but are not emitted.
  - Context usage warning is currently emitted as `ERROR` kind.
- Work:
  - Emit delta events when streaming paths are used (or remove from supported matrix if intentionally not implemented).
  - Introduce/align warning event semantics for context threshold warnings.
- Files:
  - `crates/forge-agent/src/events.rs`
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/tests/events_integration.rs`
- DoD:
  - 9.10 row in `roadmap/p24-dod-matrix.md` can be marked complete with tests.

### G5. Complete Gemini profile tool parity
- Spec refs: 3.6, 9.2
- Current gap:
  - Missing `read_many_files`, `list_dir`, and optional web tools (`web_search`, `web_fetch`).
- Work:
  - Add `list_dir` and `read_many_files` as first-class tools.
  - Decide if web tools are in-scope now or explicitly defer with matrix note.
- Files:
  - `crates/forge-agent/src/tools.rs`
  - `crates/forge-agent/src/profiles.rs`
  - `crates/forge-agent/tests/conformance_matrix.rs`
- DoD:
  - Gemini registry includes required tools or documented deferral with rationale.

### G6. Implement `AWAITING_INPUT` runtime transition logic
- Spec refs: 2.3, 9.1
- Current gap:
  - State exists but runtime loop never transitions into it.
- Work:
  - Define and implement deterministic condition for “model asks user question”.
  - Transition to `AWAITING_INPUT` and back to `PROCESSING` on next user answer.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Test asserts explicit state transitions for question/answer flow.

## Priority 2 (Quality and edge cases)

### G7. Improve `read_file` multimodal/binary behavior
- Spec refs: 3.3 `read_file`
- Current gap:
  - `read_file` assumes UTF-8 text and fails on binary/image files.
- Work:
  - Detect binary/image input and return model-usable content or clear structured error.
- Files:
  - `crates/forge-agent/src/execution.rs`
  - `crates/forge-agent/src/tools.rs`
  - tests in `crates/forge-agent/src/execution.rs` and/or integration
- DoD:
  - Binary/image read path is deterministic and tested.

### G8. Add fuzzy fallback for `edit_file` / `apply_patch` mismatch recovery
- Spec refs: 3.3 `edit_file` behavior, Appendix A hunk matching
- Current gap:
  - Matching is exact only.
- Work:
  - Add bounded fuzzy fallback (whitespace normalization first).
  - Preserve clear failure messages when ambiguity remains.
- Files:
  - `crates/forge-agent/src/tools.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Fuzzy fallback succeeds on targeted fixtures without regressing exact-match determinism.

## Cross-provider test gaps to add
- Add missing parity case from 9.12:
  - Multi-step task (`read -> analyze -> edit`) across OpenAI/Anthropic/Gemini.
- File:
  - `crates/forge-agent/tests/conformance_matrix.rs`

## Execution order
1. G1 timeout wiring
2. G2 shutdown/abort
3. G3 subagent semantics
4. G4 events
5. G5 Gemini tool parity
6. G6 awaiting-input lifecycle
7. G7 binary/image read handling
8. G8 fuzzy matching
9. Add missing 9.12 parity test

## Exit criteria for this file
- `roadmap/p24-dod-matrix.md` has no unchecked items except explicitly deferred real-key smoke notes.
- `cargo test -p forge-agent` passes with added conformance coverage.
- This file marked complete once all non-deferred gaps are closed.
