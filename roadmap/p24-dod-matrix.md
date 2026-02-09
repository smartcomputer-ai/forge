# P24 DoD Matrix (Spec Section 9)

**Status**
- In progress (2026-02-09)

This matrix is populated as implementation milestones complete.

## 9.1 Core Loop
- [x] Session creation with ProviderProfile + ExecutionEnvironment (`crates/forge-agent/src/session.rs`)
- [x] `process_input()` loop implemented (LLM call -> tool execution -> repeat) (`crates/forge-agent/src/session.rs`)
- [x] Natural completion on text-only response (`crates/forge-agent/src/session.rs`)
- [x] Per-input round limits enforced (`crates/forge-agent/src/session.rs`)
- [x] Session-level turn limits enforced (`crates/forge-agent/src/session.rs`)
- [x] Abort handling closes loop and transitions state (`crates/forge-agent/src/session.rs`)
- [x] Loop detection warning steering turn (`crates/forge-agent/src/session.rs`)
- [x] Multiple sequential inputs supported (`crates/forge-agent/src/session.rs`)

## 9.2 Provider Profiles
- [x] OpenAI profile with codex-aligned tools (`apply_patch`) (`crates/forge-agent/src/profiles.rs`, `crates/forge-agent/src/tools.rs`)
- [x] Anthropic profile with Claude-aligned tools (`edit_file`) (`crates/forge-agent/src/profiles.rs`, `crates/forge-agent/src/tools.rs`)
- [ ] Gemini profile with gemini-cli-aligned tools (deferred; see `roadmap/p24-gaps.md` G5)
- [x] Provider-specific base system prompts in place (`crates/forge-agent/src/profiles.rs`)
- [x] Custom tool extension supported (`crates/forge-agent/src/tools.rs`, `crates/forge-agent/src/profiles.rs`)
- [x] Tool name collision behavior is latest-wins (`crates/forge-agent/src/tools.rs`)

## 9.3 Tool Execution
- [x] Registry-based dispatch (`crates/forge-agent/src/tools.rs`)
- [x] Unknown tools returned as tool error results (`crates/forge-agent/src/tools.rs`)
- [x] JSON argument parsing + schema validation (`crates/forge-agent/src/tools.rs`)
- [x] Tool execution exceptions returned as `is_error=true` (`crates/forge-agent/src/tools.rs`)
- [x] Parallel execution works where supported (`crates/forge-agent/src/tools.rs`)

## 9.4 Execution Environment
- [x] LocalExecutionEnvironment implements required operations (`crates/forge-agent/src/execution.rs`)
- [x] Default timeout is 10s (`crates/forge-agent/src/execution.rs`)
- [x] Per-call timeout override supported (`crates/forge-agent/src/execution.rs`)
- [x] Timeout escalation SIGTERM -> 2s wait -> SIGKILL (`crates/forge-agent/src/execution.rs`)
- [x] Sensitive env var filtering defaults applied (`crates/forge-agent/src/execution.rs`)
- [x] Custom environment implementations possible via interface (`crates/forge-agent/src/execution.rs`)

## 9.5 Tool Output Truncation
- [x] Character truncation runs first for all outputs (`crates/forge-agent/src/truncation.rs`)
- [x] Line truncation runs second when configured (`crates/forge-agent/src/truncation.rs`)
- [x] Warning marker includes removed amount (`crates/forge-agent/src/truncation.rs`)
- [x] `TOOL_CALL_END` includes full untruncated output (`crates/forge-agent/src/tools.rs`)
- [x] Default tool character limits match spec (`crates/forge-agent/src/config.rs`)
- [x] Limits overridable via config (`crates/forge-agent/src/config.rs`)

## 9.6 Steering
- [x] `steer()` queue injection between tool rounds (`crates/forge-agent/src/session.rs`)
- [x] `follow_up()` queue processed post-completion (`crates/forge-agent/src/session.rs`)
- [x] Steering turns persisted in history (`crates/forge-agent/src/session.rs`)
- [x] Steering turns converted to user-role messages (`crates/forge-agent/src/session.rs`)

## 9.7 Reasoning Effort
- [x] `reasoning_effort` passed to request (`crates/forge-agent/src/session.rs`)
- [x] Mid-session changes apply on next call (`crates/forge-agent/src/session.rs`)
- [x] Supported values covered by validation/tests (`crates/forge-agent/src/session.rs`)

## 9.8 System Prompts
- [x] Includes provider base instructions (`crates/forge-agent/src/profiles.rs`)
- [x] Includes environment context block (`crates/forge-agent/src/profiles.rs`, `crates/forge-agent/src/session.rs`)
- [x] Includes active tool descriptions (`crates/forge-agent/src/profiles.rs`)
- [x] Includes discovered project docs (`crates/forge-agent/src/session.rs`)
- [x] User override appended last (`crates/forge-agent/src/profiles.rs`, `crates/forge-agent/src/config.rs`)
- [x] Provider-relevant docs only (plus `AGENTS.md`) (`crates/forge-agent/src/profiles.rs`, `crates/forge-agent/src/session.rs`)

## 9.9 Subagents
- [x] `spawn_agent` implemented (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/tools.rs`)
- [x] Shared execution environment behavior (`crates/forge-agent/src/session.rs`)
- [x] Independent history per subagent (`crates/forge-agent/src/session.rs`)
- [x] Depth limiting enforced (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/config.rs`)
- [x] Subagent result returned to parent (`crates/forge-agent/src/session.rs`)
- [x] `send_input`, `wait`, `close_agent` implemented (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/tools.rs`)

## 9.10 Event System
- [x] All Section 2.9 event kinds emitted (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/tools.rs`, `crates/forge-agent/tests/events_integration.rs`)
- [x] Events consumable via async stream/iterator (`crates/forge-agent/src/events.rs`)
- [x] `TOOL_CALL_END` carries full output (`crates/forge-agent/src/tools.rs`)
- [x] `SESSION_START` and `SESSION_END` emitted correctly (`crates/forge-agent/src/session.rs`)

## 9.11 Error Handling
- [x] Tool errors return recoverable tool results (`crates/forge-agent/src/tools.rs`)
- [x] Transient provider errors rely on SDK retry layer (`crates/forge-agent/src/session.rs`, `crates/forge-llm/src/client.rs`)
- [x] Authentication errors close session without retry (`crates/forge-agent/src/session.rs`)
- [x] Context usage warnings emitted (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/events.rs`)
- [x] Graceful shutdown sequence implemented (`crates/forge-agent/src/session.rs`, `crates/forge-agent/src/execution.rs`, `crates/forge-agent/src/session.rs` tests)

## 9.12 Cross-Provider Parity Matrix
- Note: Gemini parity expansion is deferred for current iteration.
- [x] Simple file creation task (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Read then edit task (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Multi-file edit flow (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Shell execution flow (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Shell timeout handling (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Grep + glob discovery flow (`crates/forge-agent/tests/conformance_matrix.rs`)
- [ ] Multi-step read/analyze/edit
- [x] Large-output truncation behavior (`crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Parallel tool calls (where supported) (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Mid-task steering behavior (`crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Reasoning effort change behavior (`crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Subagent spawn and wait flow (`crates/forge-agent/tests/conformance_matrix.rs`)
- [x] Loop detection warning behavior (`crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Error recovery after tool failure (`crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Provider-native editing format behavior (`crates/forge-agent/tests/conformance_matrix.rs`)

## 9.13 Integration Smoke Test
- [x] OpenAI smoke scenario complete (mocked) (`crates/forge-agent/tests/conformance_matrix.rs`, `crates/forge-agent/tests/events_integration.rs`)
- [x] Anthropic smoke scenario complete (mocked) (`crates/forge-agent/tests/conformance_matrix.rs`, `crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [x] Gemini smoke scenario complete (mocked) (`crates/forge-agent/tests/conformance_matrix.rs`, `crates/forge-agent/tests/conformance_runtime_behaviors.rs`)
- [ ] Real-key run notes captured
