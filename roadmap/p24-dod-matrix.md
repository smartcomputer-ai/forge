# P24 DoD Matrix (Spec Section 9)

**Status**
- Planned (2026-02-09)

This matrix is populated as implementation milestones complete.

## 9.1 Core Loop
- [ ] Session creation with ProviderProfile + ExecutionEnvironment
- [ ] `process_input()` loop implemented (LLM call -> tool execution -> repeat)
- [ ] Natural completion on text-only response
- [ ] Per-input round limits enforced
- [ ] Session-level turn limits enforced
- [ ] Abort handling closes loop and transitions state
- [ ] Loop detection warning steering turn
- [ ] Multiple sequential inputs supported

## 9.2 Provider Profiles
- [ ] OpenAI profile with codex-aligned tools (`apply_patch`)
- [ ] Anthropic profile with Claude-aligned tools (`edit_file`)
- [ ] Gemini profile with gemini-cli-aligned tools
- [ ] Provider-specific base system prompts in place
- [ ] Custom tool extension supported
- [ ] Tool name collision behavior is latest-wins

## 9.3 Tool Execution
- [ ] Registry-based dispatch
- [ ] Unknown tools returned as tool error results
- [ ] JSON argument parsing + schema validation
- [ ] Tool execution exceptions returned as `is_error=true`
- [ ] Parallel execution works where supported

## 9.4 Execution Environment
- [ ] LocalExecutionEnvironment implements required operations
- [ ] Default timeout is 10s
- [ ] Per-call timeout override supported
- [ ] Timeout escalation SIGTERM -> 2s wait -> SIGKILL
- [ ] Sensitive env var filtering defaults applied
- [ ] Custom environment implementations possible via interface

## 9.5 Tool Output Truncation
- [ ] Character truncation runs first for all outputs
- [ ] Line truncation runs second when configured
- [ ] Warning marker includes removed amount
- [ ] `TOOL_CALL_END` includes full untruncated output
- [ ] Default tool character limits match spec
- [ ] Limits overridable via config

## 9.6 Steering
- [ ] `steer()` queue injection between tool rounds
- [ ] `follow_up()` queue processed post-completion
- [ ] Steering turns persisted in history
- [ ] Steering turns converted to user-role messages

## 9.7 Reasoning Effort
- [ ] `reasoning_effort` passed to request
- [ ] Mid-session changes apply on next call
- [ ] Supported values covered by validation/tests

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
- [ ] Events consumable via async stream/iterator
- [ ] `TOOL_CALL_END` carries full output
- [ ] `SESSION_START` and `SESSION_END` emitted correctly

## 9.11 Error Handling
- [ ] Tool errors return recoverable tool results
- [ ] Transient provider errors rely on SDK retry layer
- [ ] Authentication errors close session without retry
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
