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
- [ ] OpenAI profile with codex-aligned tools (`apply_patch`)
- [ ] Anthropic profile with Claude-aligned tools (`edit_file`)
- [ ] Gemini profile with gemini-cli-aligned tools
- [ ] Provider-specific base system prompts in place
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
- [ ] Includes provider base instructions
- [ ] Includes environment context block
- [ ] Includes active tool descriptions
- [ ] Includes discovered project docs
- [ ] User override appended last
- [ ] Provider-relevant docs only (plus `AGENTS.md`)

## 9.9 Subagents
- [ ] `spawn_agent` implemented
- [ ] Shared execution environment behavior
- [ ] Independent history per subagent
- [ ] Depth limiting enforced
- [ ] Subagent result returned to parent
- [ ] `send_input`, `wait`, `close_agent` implemented

## 9.10 Event System
- [ ] All Section 2.9 event kinds emitted
- [x] Events consumable via async stream/iterator (`crates/forge-agent/src/events.rs`)
- [x] `TOOL_CALL_END` carries full output (`crates/forge-agent/src/tools.rs`)
- [x] `SESSION_START` and `SESSION_END` emitted correctly (`crates/forge-agent/src/session.rs`)

## 9.11 Error Handling
- [x] Tool errors return recoverable tool results (`crates/forge-agent/src/tools.rs`)
- [x] Transient provider errors rely on SDK retry layer (`crates/forge-agent/src/session.rs`, `crates/forge-llm/src/client.rs`)
- [x] Authentication errors close session without retry (`crates/forge-agent/src/session.rs`)
- [ ] Context usage warnings emitted
- [ ] Graceful shutdown sequence implemented

## 9.12 Cross-Provider Parity Matrix
- [ ] Simple file creation task
- [ ] Read then edit task
- [ ] Multi-file edit flow
- [ ] Shell execution flow
- [ ] Shell timeout handling
- [ ] Grep + glob discovery flow
- [ ] Multi-step read/analyze/edit
- [ ] Large-output truncation behavior
- [ ] Parallel tool calls (where supported)
- [ ] Mid-task steering behavior
- [ ] Reasoning effort change behavior
- [ ] Subagent spawn and wait flow
- [ ] Loop detection warning behavior
- [ ] Error recovery after tool failure
- [ ] Provider-native editing format behavior

## 9.13 Integration Smoke Test
- [ ] OpenAI smoke scenario complete
- [ ] Anthropic smoke scenario complete
- [ ] Gemini smoke scenario complete
- [ ] Real-key run notes captured
