# P25: Agent Backend Readiness for Attractor (Spec 03)

**Status**
- In progress (2026-02-09)

**Goal**
Close `forge-agent` integration gaps that block or complicate a faithful implementation of `spec/03-attractor-spec.md` codergen backend behavior.

**Source**
- Spec of record: `spec/03-attractor-spec.md`
- Agent baseline: `spec/02-coding-agent-loop-spec.md` + `roadmap/p24-dod-matrix.md`

**Context**
- `forge-agent` is already a strong runtime for multi-turn coding loops.
- Attractor needs a codergen backend surface that can control per-node execution policy, observe/annotate tool activity, and persist/resume deterministic state across pipeline runs.
- This roadmap covers `forge-agent` changes only (not DOT parser/runner work).

## Priority 0 (Must-have for clean Attractor backend integration)

### [x] G1. Add tool hook extension points (`tool_hooks.pre` / `tool_hooks.post`)
- Spec refs: 9.7, Appendix A (`tool_hooks.pre`, `tool_hooks.post`)
- Current gap:
  - Tool dispatch executes immediately after validation with no pre/post callback/hook seam.
  - Subagent tools are executed via a separate path, so registry wrapping alone is insufficient.
- Work:
  - Add hook interfaces invoked around every tool call (registry + subagent tools).
  - Support pre-hook outcome: allow, skip, or fail-with-message (for policy engines).
  - Include structured tool metadata in hook context (tool name, call id, args, session id).
  - Emit warning/error events when hooks fail without crashing session loop unless configured strict.
- Files:
  - `crates/forge-agent/src/tools/registry.rs`
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/events.rs`
  - `crates/forge-agent/src/config.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Pre-hook can deterministically skip a tool call and return structured tool result.
  - Post-hook receives success/error result for both regular and subagent tools.
  - Hook failures are observable in events and tests.
- Completed:
  - Added `ToolCallHook` interface with `before_tool_call` / `after_tool_call`.
  - Added `ToolPreHookOutcome` (`Continue`, `Skip`, `Fail`) and structured hook context models.
  - Wired hooks through both registry-dispatched tools and session-managed subagent tools.
  - Added strict/non-strict behavior via `SessionConfig.tool_hook_strict`.
  - Added warning/error event emission for hook failures and hook-driven skips.
  - Added unit coverage for regular + subagent hook behavior.

### [x] G2. Add per-submit request overrides (provider/model/system prompt/reasoning)
- Spec refs: 2.6, 8.5, 9.2, 11.10 (node-level model/provider/reasoning control)
- Current gap:
  - Session requests always use static `provider_profile` model/provider at construction time.
  - Only mid-session `reasoning_effort` mutation exists globally; no per-turn override object.
- Work:
  - Introduce a `SubmitOptions`/`RequestOverrides` input for `submit(...)`.
  - Allow one-turn overrides for:
    - `model`
    - `provider` profile key (or pre-registered profile switch)
    - `reasoning_effort`
    - `system_prompt_override`
    - optional request metadata/provider options
  - Keep backward compatibility with current `submit(user_input)` API.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/profiles.rs`
  - `crates/forge-agent/src/lib.rs`
  - `crates/forge-agent/tests/conformance_matrix.rs`
- DoD:
  - Attractor codergen adapter can set node-level provider/model/reasoning without recreating session each node.
  - Existing spec/02 behavior remains unchanged when no overrides are provided.
- Completed:
  - Added `SubmitOptions` and `Session::submit_with_options(...)`.
  - Added one-turn overrides for `provider`, `model`, `reasoning_effort`, `system_prompt_override`, `provider_options`, and request metadata.
  - Added provider-profile registration API (`register_provider_profile`) for pre-registered profile switching.
  - Kept existing `submit(...)` behavior as default/no-overrides path.
  - Added unit coverage validating provider/model/reasoning/system-prompt override behavior.

### [x] G3. Add checkpoint snapshot/restore API for session state
- Spec refs: 5.3, 5.4 resume note, 11.7
- Current gap:
  - Session history is readable/pushable but has no first-class checkpoint serialization contract.
  - Resume logic must currently be implemented by external code manually and is brittle.
- Work:
  - Add serializable `SessionCheckpoint` model:
    - session id
    - state
    - history
    - queues (`steering`, `follow_up`)
    - config snapshot
    - minimal subagent terminal summaries (or explicit unsupported marker)
  - Add `Session::checkpoint()` and `Session::from_checkpoint(...)`.
  - Define explicit behavior for non-serializable runtime parts (event emitter, live tasks, abort handles).
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/turn.rs`
  - `crates/forge-agent/src/errors.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Round-trip checkpoint restore reproduces deterministic next-turn behavior in tests.
  - Non-resumable in-flight subagent state is handled explicitly (clear error or safe degrade).
- Completed:
  - Added serializable `SessionCheckpoint`.
  - Added `Session::checkpoint()` and `Session::from_checkpoint(...)`.
  - Checkpoint includes session id/state/history/queues/config.
  - Added explicit unsupported error (`SessionError::CheckpointUnsupported`) when checkpointing with active subagent tasks.
  - Added unit coverage for round-trip restore and active-subagent rejection behavior.

## Priority 1 (Strongly recommended to reduce adapter glue)

### [ ] G4. Add structured submit outcome API
- Spec refs: 4.5 codergen backend contract, 5.2 Outcome, Appendix C status contract
- Current gap:
  - `submit(...)` returns `Result<(), AgentError>`; callers must inspect history/events to infer last output.
- Work:
  - Introduce `SubmitResult` including:
    - terminal session state after submit
    - assistant text emitted this submit
    - tool-call summary (count + error count + ids)
    - usage summary (if available)
  - Keep existing method and add `submit_with_result(...)` to avoid breaking changes.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/lib.rs`
  - `crates/forge-agent/tests/events_integration.rs`
- DoD:
  - Codergen backend can map `SubmitResult` to Attractor `Outcome` without replaying entire history.

### [ ] G5. Enrich tool-call event payloads for external orchestration
- Spec refs: 9.6 events, 9.7 tool hook metadata needs
- Current gap:
  - `TOOL_CALL_START` includes only name + call id; no arguments payload.
  - No duration/attempt metadata in `TOOL_CALL_END`.
- Work:
  - Add optional structured event fields:
    - `arguments` on start
    - `duration_ms`, `is_error` on end
  - Preserve current event kinds; additive payload only.
- Files:
  - `crates/forge-agent/src/events.rs`
  - `crates/forge-agent/src/tools/registry.rs`
  - `crates/forge-agent/tests/events_integration.rs`
- DoD:
  - Attractor runtime can project agent events into stage telemetry without parsing freeform text.

## Priority 2 (Nice-to-have hardening)

### [ ] G6. Add session thread key metadata for `full` fidelity interop
- Spec refs: 5.4 thread resolution
- Current gap:
  - Thread/session reuse key is implicit in session instance identity; no explicit external thread key.
- Work:
  - Add optional session metadata field (`thread_key`) and accessors.
  - Carry thread key through checkpoints and submit results.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/config.rs`
  - `crates/forge-agent/tests/conformance_runtime_behaviors.rs`
- DoD:
  - Attractor can persist and reason about thread continuity across node runs.

## Test additions (cross-cutting)
- [ ] Add regression tests for hook execution order and skip/fail behavior.
- [ ] Add per-submit override tests for model/provider/reasoning/system prompt.
- [ ] Add checkpoint round-trip tests (including restored queues).
- [ ] Add submit-result contract tests.
- [ ] Add event payload backward-compatibility tests (existing consumers unaffected).

## Execution order
1. G1 tool hooks
2. G2 per-submit overrides
3. G3 checkpoint/restore
4. G4 submit result API
5. G5 event payload enrichment
6. G6 thread key metadata

## Out of scope for P25
- DOT parsing, validation, transforms, and pipeline traversal engine (`spec/03` Sections 2, 3, 7, 9).
- Interviewer implementations and HTTP server mode.
- Attractor-specific graph/run data model crate.

## Exit criteria for this file
- `forge-agent` exposes the minimal API surface needed for a low-glue Attractor codergen backend.
- New APIs are additive and do not regress `spec/02` conformance tests.
- `cargo test -p forge-agent` passes with new coverage for hooks, overrides, and checkpoints.
