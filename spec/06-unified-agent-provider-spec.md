# Unified Agent Provider Specification

This document specifies the architecture for provider-owned agent loops -- a refactoring of the coding agent system where each provider owns its complete agent cycle (prompt → tool execution → iteration → final answer), rather than the Session always driving the tool loop externally.

This spec layers on top of the [Unified LLM Client Specification](./01-unified-llm-spec.md) and the [Coding Agent Loop Specification](./02-coding-agent-loop-spec.md). It does not replace those specs; it restructures the boundary between the LLM client layer and the agent session layer.

---

## Table of Contents

1. [Overview and Goals](#1-overview-and-goals)
2. [Architecture](#2-architecture)
3. [AgentProvider Contract](#3-agentprovider-contract)
4. [HTTP API Agent Provider](#4-http-api-agent-provider)
5. [CLI Agent Providers](#5-cli-agent-providers)
6. [Session Integration](#6-session-integration)
7. [Provider Configuration](#7-provider-configuration)
8. [Event Observability](#8-event-observability)
9. [Definition of Done](#9-definition-of-done)

---

## 1. Overview and Goals

### 1.1 Problem Statement

The current coding agent architecture (spec/02) implements the agentic tool loop in the Session layer. The Session calls `Client.complete()`, extracts tool calls from the response, dispatches them through a `ToolRegistry` against an `ExecutionEnvironment`, feeds results back into the conversation, and repeats. This works for raw HTTP API providers (OpenAI, Anthropic, Gemini) where the LLM is a stateless completion endpoint.

However, modern coding agent CLIs -- Claude Code, Codex CLI, Gemini CLI -- are complete agent systems that manage their own tool loops internally. Each ships with:

1. **Model-trained tools.** The underlying models were specifically trained against these tool schemas. Claude was trained on `text_editor_20250728` and `web_search_20250305`. Codex models were trained against the `shell` tool and apply_patch format. These trained tools produce higher-quality tool calls than generic reimplementations.

2. **Battle-tested system prompts.** Hundreds of lines of carefully crafted instructions co-evolved with model training. Safety constraints, tool usage patterns, error recovery, permission models.

3. **Context management.** Compaction, truncation, and context window strategies tuned to the specific model family.

4. **Built-in sandboxing.** Permission flows, filesystem isolation (Landlock, bubblewrap), command approval gates.

The Session's external tool loop cannot access these provider-internal capabilities. To leverage them, the provider must own the complete agent cycle.

### 1.2 Design Principles

**Unified abstraction.** Every provider -- HTTP API or CLI subprocess -- exposes the same interface: prompt in, completed result out. The Session does not need to know which kind of provider is running.

**Extract, don't duplicate.** The existing tool loop in Session is extracted into an HTTP API agent provider, not rewritten. The logic is identical; only its location changes.

**Provider-opaque tool execution.** When a CLI agent provider runs, the Session observes tool calls for event emission and history recording, but does not execute them. The provider did that internally.

**Explicit configuration.** Providers are configured explicitly. The system does not auto-discover providers from environment variables or PATH scanning. Users declare which providers they want to use.

**Backward compatible.** The HTTP API agent provider runs the same code that currently lives in Session. Existing behavior is preserved exactly. The refactor is a no-op from the perspective of tests and end users -- until they configure a CLI provider.

### 1.3 Architecture

```
+--------------------------------------------------+
|  Host Application (CLI, IDE, Web UI, Pipeline)    |
+--------------------------------------------------+
        |                            ^
        | submit(input)              | events
        v                            |
+--------------------------------------------------+
|  Session                                          |
|  - conversation history                           |
|  - state machine (Idle/Processing/Closed)         |
|  - event emission                                 |
|  - persistence                                    |
|  - delegates to AgentProvider                     |
+--------------------------------------------------+
        |
        v (unified interface)
+--------------------------------------------------+
|  AgentProvider                                    |
|  run_to_completion(prompt, options) -> result     |
+--------------------------------------------------+
        |                            |
        v                            v
+---------------------+   +---------------------+
| HttpApiAgentProvider |   | CLI Agent Providers |
| (extracted loop)     |   | (subprocess)        |
|                      |   |                     |
| ProviderAdapter      |   | claude --print      |
| + ToolRegistry       |   | codex exec --json   |
| + ExecutionEnv       |   | gemini --jsonl      |
+---------------------+   +---------------------+
        |
        v
+--------------------------------------------------+
|  Unified LLM SDK (Client.complete / stream)       |
+--------------------------------------------------+
```

The key change from spec/02's architecture: the `Session` no longer calls `Client.complete()` directly. It calls `AgentProvider.run_to_completion()`, which is implemented either by wrapping `Client.complete()` + tool dispatch (HTTP path) or by spawning a CLI subprocess (CLI path).

---

## 2. Architecture

### 2.1 Component Relationships

```
forge-llm (no changes to existing code)
├── ProviderAdapter trait        -- stateless LLM call (unchanged)
├── Client                       -- provider routing + middleware (unchanged)
├── AgentProvider trait           -- NEW: unified agent loop contract
├── cli_adapters/
│   ├── ClaudeCodeAgentProvider  -- NEW: spawns claude CLI
│   ├── CodexAgentProvider       -- NEW: spawns codex CLI
│   └── GeminiAgentProvider      -- NEW: spawns gemini CLI

forge-agent
├── HttpApiAgentProvider         -- NEW: extracted tool loop
├── Session                      -- SIMPLIFIED: delegates to AgentProvider
├── ProviderProfile              -- EXTENDED: agent_provider_name()
├── ToolRegistry                 -- unchanged (used by HttpApiAgentProvider)
├── ExecutionEnvironment         -- unchanged (used by HttpApiAgentProvider)
```

### 2.2 Dependency Flow

The `AgentProvider` trait lives in `forge-llm` to avoid circular dependencies:

```
forge-llm defines: AgentProvider trait, CLI adapters
forge-agent defines: HttpApiAgentProvider (implements AgentProvider, depends on forge-llm)
forge-agent's Session: holds Arc<dyn AgentProvider>, calls run_to_completion()
```

`forge-llm` remains independent of `forge-agent`. `forge-agent` depends on `forge-llm` as before. No circular dependencies.

---

## 3. AgentProvider Contract

### 3.1 Core Trait

```
TRAIT AgentProvider:
    -- Human-readable name for this provider.
    FUNCTION name() -> String

    -- Run the complete agent loop: prompt in, final answer out.
    -- The provider handles all tool execution internally.
    ASYNC FUNCTION run_to_completion(
        prompt          : String,
        options         : AgentRunOptions
    ) -> AgentRunResult | Error
```

### 3.2 Options

```
RECORD AgentRunOptions:
    working_directory       : Path              -- where the agent operates
    model_override          : String | None      -- override the default model
    max_turns               : Integer | None     -- max LLM call rounds
    max_tool_rounds         : Integer | None     -- max tool execution rounds
    reasoning_effort        : String | None      -- "low", "medium", "high"
    system_prompt_override  : String | None      -- override system prompt
    env_vars                : Map<String, String> | None  -- environment for subprocess/tools
    on_event                : EventCallback | None -- real-time event observation
```

The `on_event` callback allows the Session (or any caller) to observe events in real time without polling. This is how the Session emits `SessionEvent` notifications during provider-managed loops.

### 3.3 Result

```
RECORD AgentRunResult:
    text                    : String             -- final text response
    tool_activity           : List<ToolActivityRecord>  -- what tools ran (observability)
    usage                   : Usage              -- aggregated token usage
    id                      : String             -- response/session ID
    model                   : String             -- model that was used
    provider                : String             -- provider name
    cost_usd                : Float | None       -- total cost if known
    duration_ms             : Integer | None      -- wall clock time
```

### 3.4 Tool Activity Record

```
RECORD ToolActivityRecord:
    tool_name               : String
    call_id                 : String
    arguments_summary       : String | None      -- truncated for observability
    result_summary          : String | None      -- truncated for observability
    is_error                : Boolean
    duration_ms             : Integer | None
```

Tool activity records are informational. They let the Session record what happened for history, events, and persistence, without the Session having to execute the tools.

---

## 4. HTTP API Agent Provider

### 4.1 Purpose

The `HttpApiAgentProvider` implements `AgentProvider` by composing the existing `ProviderAdapter` (from forge-llm), `ToolRegistry`, and `ExecutionEnvironment` (from forge-agent). It runs the **same tool loop** that currently lives in `Session.submit_single()` (spec/02, section 2.5).

### 4.2 Construction

```
RECORD HttpApiAgentProvider:
    llm_client          : Client              -- from forge-llm
    provider_profile    : ProviderProfile     -- tools, system prompt, capabilities
    execution_env       : ExecutionEnvironment -- where tools run
    config              : SessionConfig       -- limits, timeouts
```

### 4.3 run_to_completion Implementation

This is a direct extraction of the loop from spec/02, section 2.5:

```
FUNCTION run_to_completion(prompt, options) -> AgentRunResult:
    messages = [Message.user(prompt)]
    tool_activity = []
    total_usage = Usage.zero()
    round_count = 0

    -- Build system prompt using provider profile
    system_prompt = options.system_prompt_override
        OR provider_profile.build_system_prompt(environment, project_docs)

    LOOP:
        IF round_count >= options.max_tool_rounds:
            BREAK

        -- Build request
        request = Request(
            model = options.model_override OR provider_profile.model,
            messages = [Message.system(system_prompt)] + messages,
            tools = provider_profile.tools(),
            tool_choice = "auto",
            reasoning_effort = options.reasoning_effort
        )

        -- Call LLM
        response = llm_client.complete(request)
        total_usage += response.usage

        -- Record assistant message
        messages.APPEND(Message.assistant(response.text, response.tool_calls))

        -- If no tool calls, done
        IF response.tool_calls IS EMPTY:
            RETURN AgentRunResult(
                text = response.text,
                tool_activity = tool_activity,
                usage = total_usage,
                ...
            )

        -- Execute tools
        round_count += 1
        FOR EACH tool_call IN response.tool_calls:
            result = tool_registry.dispatch_single(tool_call, execution_env)
            tool_activity.APPEND(ToolActivityRecord from result)
            messages.APPEND(Message.tool_result(tool_call.id, result))
            options.on_event?(ToolCallEnd event)

        -- Loop detection (same logic as spec/02 section 2.5 step 8)
        IF detect_loop(tool_activity):
            messages.APPEND(Message.user(loop_warning))

    END LOOP

    RETURN AgentRunResult(text = last_text, ...)
```

### 4.4 Behavioral Equivalence

The `HttpApiAgentProvider` must produce identical results to the current `Session.submit_single()` for the same inputs. The refactor is a no-op. Verification: all existing tests pass without modification.

---

## 5. CLI Agent Providers

### 5.1 General Pattern

Each CLI agent provider spawns its respective CLI binary as a child process, passes the prompt, parses the JSONL output stream, and returns the result.

```
FUNCTION run_to_completion(prompt, options) -> AgentRunResult:
    cmd = build_command(prompt, options)
    child = spawn(cmd, cwd = options.working_directory, env = options.env_vars)

    tool_activity = []
    final_text = ""
    total_usage = Usage.zero()

    FOR EACH line IN child.stdout:
        event = parse_jsonl(line)
        MATCH event.type:
            "assistant" | "agent_message":
                -- Extract tool_use blocks for observability
                FOR EACH tool_use IN event.tool_uses:
                    tool_activity.APPEND(ToolActivityRecord from tool_use)
                    options.on_event?(ToolCallStart/ToolCallEnd events)
                options.on_event?(TextDelta event with text content)

            "result" | "turn.completed":
                final_text = event.result_text
                total_usage = event.usage

            -- Provider-specific events: stream through on_event

    wait_for_exit(child)
    RETURN AgentRunResult(text = final_text, tool_activity, usage, ...)
```

### 5.2 Claude Code Provider

**Binary:** `claude` (configurable path)

**Invocation:**
```
claude -p "<prompt>" --output-format stream-json --verbose [--model <model>] [--max-turns <n>]
```

**JSONL output types:**

| Type | Purpose | Key fields |
|------|---------|------------|
| `system` | Session init | `session_id`, `tools[]`, `model` |
| `assistant` | Agent response | `message.content[]` with `text` and `tool_use` blocks |
| `user` | Tool results | `message.content[]` with `tool_result` blocks |
| `result` | Final result | `result` (text), `usage`, `total_cost_usd`, `num_turns` |

**Tool use extraction:** Each `assistant` event's `message.content` array contains `tool_use` blocks with `name`, `id`, and `input` fields. These become `ToolActivityRecord` entries.

**Cost tracking:** The `result` event includes `total_cost_usd`.

### 5.3 Codex Provider

**Binary:** `codex` (configurable path)

**Invocation:**
```
codex exec --json "<prompt>" [--model <model>]
```

**JSONL output types:**

| Type | Purpose | Key fields |
|------|---------|------------|
| `thread.started` | Session init | `thread_id` |
| `item.started` | Item begin | `item.type` (command_execution, agent_message, file_change) |
| `item.completed` | Item done | Full item with results |
| `turn.completed` | Turn done | `usage` |

**Tool use extraction:** `item.completed` events with `item.type == "command_execution"` or `"file_change"` map to `ToolActivityRecord`. The `item.type == "agent_message"` events contain the text output.

### 5.4 Gemini Provider

**Binary:** `gemini` (configurable path)

**Invocation:**
```
gemini --output-format jsonl "<prompt>"
```

**Tool use extraction:** Gemini's JSONL events include tool call information similar to its streaming API. Exact format TBD as the non-interactive mode is still stabilizing.

### 5.5 Subprocess Lifecycle

**Startup:** Each invocation spawns a fresh process. No long-lived subprocess management.

**Abort:** The Session's abort mechanism (`tokio::select!` against `abort_notify`) kills the child process (SIGTERM, then SIGKILL after grace period).

**Environment:** The subprocess inherits the configured environment variables. API keys for the provider must be available in the environment.

**Working directory:** Set via `Command.current_dir()` to `options.working_directory`.

**Error handling:** Non-zero exit code without a `result` event is an error. Parse errors on individual JSONL lines are logged and skipped.

---

## 6. Session Integration

### 6.1 Simplified Session

The Session gains an `AgentProvider` (or a resolver that maps provider names to providers) and delegates the tool loop entirely.

```
RECORD Session:
    id                : String
    agent_provider    : AgentProvider         -- CHANGED: was llm_client + provider_profile
    execution_env     : ExecutionEnvironment  -- still needed for environment context
    history           : List<Turn>
    event_emitter     : EventEmitter
    config            : SessionConfig
    state             : SessionState
    steering_queue    : Queue<String>
    followup_queue    : Queue<String>
    subagents         : Map<String, SubAgent>
```

### 6.2 Simplified submit_single

```
FUNCTION submit_single(session, user_input) -> Boolean:
    -- 1. Pre-flight
    IF session.state == CLOSED: ERROR
    IF session.abort_signaled: shutdown, RETURN false
    session.state = PROCESSING
    session.history.APPEND(UserTurn(content = user_input))
    session.emit(USER_INPUT)
    drain_steering(session)

    -- 2. Build options
    options = AgentRunOptions(
        working_directory = session.execution_env.working_directory(),
        model_override = submit_options.model,
        max_turns = session.config.max_turns,
        max_tool_rounds = session.config.max_tool_rounds_per_input,
        reasoning_effort = session.config.reasoning_effort,
        on_event = callback that forwards to session.event_emitter,
    )

    -- 3. Delegate to provider (with abort race)
    session.emit(ASSISTANT_TEXT_START)
    result = SELECT:
        session.agent_provider.run_to_completion(user_input, options)
            -> result
        session.abort_notify.wait()
            -> shutdown, RETURN false

    -- 4. Record result
    assistant_turn = AssistantTurn(
        content = result.text,
        tool_calls = [],  -- empty: provider handled them
        usage = result.usage,
        response_id = result.id,
        provider_tool_call_count = len(result.tool_activity),
    )
    session.history.APPEND(assistant_turn)
    session.emit(ASSISTANT_TEXT_END, text = result.text)

    -- 5. State transition
    IF looks_like_question(result.text):
        session.state = AWAITING_INPUT
    ELSE:
        session.state = IDLE

    RETURN true  -- completed naturally
```

### 6.3 AssistantTurn Extension

```
RECORD AssistantTurn:
    content                   : String
    tool_calls                : List<ToolCall>
    reasoning                 : String | None
    usage                     : Usage
    response_id               : String | None
    timestamp                 : Timestamp
    provider_tool_call_count  : Integer | None    -- NEW: tools executed by provider
```

When `provider_tool_call_count` is set, `SubmitResult.tool_call_count` uses it instead of counting `ToolResultsTurn` entries in history.

### 6.4 Conversation History

For `HttpApiAgentProvider`: the provider manages an internal message list for the LLM conversation. The Session only sees the final `AssistantTurn`. This means the detailed turn-by-turn history (User → Assistant with tool_calls → ToolResults → Assistant → ...) lives inside the provider, not in the Session's `history`.

This is a simplification: the Session's `history` becomes a sequence of `UserTurn` → `AssistantTurn` pairs, without interleaved `ToolResultsTurn` entries. The `ToolResultsTurn` type is still used internally by `HttpApiAgentProvider` but is not pushed to Session's history.

**Migration note:** Existing code that scans `session.history` for `Turn::ToolResults` will need to be updated to use `AssistantTurn.provider_tool_call_count` instead. This affects `submit_with_result()` and any persistence code that serializes history.

### 6.5 Steering and Follow-up Queues

Steering messages (`session.steer()`) are currently injected between tool rounds inside `submit_single()`. With the provider owning the loop:

- **HttpApiAgentProvider**: accepts a steering channel in `AgentRunOptions`. The provider checks the channel between tool rounds and injects messages, exactly as the current code does.
- **CLI providers**: steering is not supported (the subprocess manages its own conversation). Steering messages queued during a CLI provider's run are held until the next `submit()` call.

### 6.6 Subagents

Subagent tools (`spawn_agent`, `send_input`, `wait`, `close_agent`) are currently handled by the Session, not the ToolRegistry. With the provider owning the loop:

- **HttpApiAgentProvider**: subagent tools are handled by the provider, which has access to the Session's subagent management via a callback or shared reference.
- **CLI providers**: subagent spawning is not applicable (the CLI manages its own internal agents).

---

## 7. Provider Configuration

### 7.1 No Auto-Discovery

Providers are NOT auto-discovered from environment variables, PATH scanning, or any implicit mechanism. The host application explicitly declares which providers are available.

### 7.2 Configuration Structure

```
RECORD ProviderConfig:
    entries     : List<ProviderEntry>    -- declared providers
    default     : String | None          -- default provider name

RECORD ProviderEntry:
    name            : String             -- unique identifier (e.g., "claude-code", "anthropic")
    kind            : ProviderKind       -- type of provider
    api_key         : String | None      -- for HTTP API providers
    model           : String | None      -- default model
    binary_path     : String | None      -- for CLI providers: path to binary
    base_url        : String | None      -- for HTTP providers: API endpoint override

ENUM ProviderKind:
    ANTHROPIC_API       -- Anthropic Messages API (HTTP)
    OPENAI_API          -- OpenAI Responses/Chat Completions API (HTTP)
    CLAUDE_CODE_CLI     -- Claude Code CLI subprocess
    CODEX_CLI           -- Codex CLI subprocess
    GEMINI_CLI          -- Gemini CLI subprocess
```

### 7.3 Provider Construction

The host application reads `ProviderConfig` and constructs the appropriate `AgentProvider` for each entry:

- `ANTHROPIC_API` / `OPENAI_API` → `HttpApiAgentProvider` wrapping the corresponding `ProviderAdapter`
- `CLAUDE_CODE_CLI` → `ClaudeCodeAgentProvider`
- `CODEX_CLI` → `CodexAgentProvider`
- `GEMINI_CLI` → `GeminiAgentProvider`

The Session receives the constructed `AgentProvider`. It does not know how to construct providers.

---

## 8. Event Observability

### 8.1 Event Callback

The `AgentRunOptions.on_event` callback delivers real-time events from the provider to the Session (or any observer).

```
ENUM AgentLoopEvent:
    TextDelta:
        delta           : String

    ToolCallStart:
        call_id         : String
        tool_name       : String
        arguments       : JSON

    ToolCallEnd:
        call_id         : String
        output          : String
        is_error        : Boolean
        duration_ms     : Integer

    Warning:
        message         : String

    ContextUsage:
        approx_tokens   : Integer
        context_window   : Integer
        usage_percent   : Float
```

### 8.2 Event Flow

```
AgentProvider emits AgentLoopEvent via on_event callback
    ↓
Session translates to SessionEvent and emits via EventEmitter
    ↓
Host application receives SessionEvent
```

The Session's event types (from spec/02) are unchanged. The translation is:

| AgentLoopEvent | SessionEvent |
|----------------|-------------|
| TextDelta | AssistantTextDelta |
| ToolCallStart | ToolCallStart |
| ToolCallEnd | ToolCallEnd |
| Warning | Warning |
| ContextUsage | Warning (context usage) |

---

## 9. Definition of Done

### 9.1 Trait and Types

- [ ] `AgentProvider` trait defined in `forge-llm` with `name()` and `run_to_completion()`
- [ ] `AgentRunOptions`, `AgentRunResult`, `ToolActivityRecord`, `AgentLoopEvent` types defined
- [ ] All types implement `Clone`, `Debug`, and serde `Serialize`/`Deserialize` where appropriate

### 9.2 CLI Agent Providers

- [ ] `ClaudeCodeAgentProvider` implements `AgentProvider`, spawns `claude` CLI, parses JSONL output
- [ ] `CodexAgentProvider` implements `AgentProvider`, spawns `codex` CLI, parses JSONL output
- [ ] `GeminiAgentProvider` implements `AgentProvider`, spawns `gemini` CLI, parses JSONL output
- [ ] Each adapter handles: subprocess spawn, JSONL parsing, tool activity extraction, usage/cost collection, error handling, clean exit
- [ ] Unit tests for each adapter with recorded JSONL fixtures (no actual CLI binary required)

### 9.3 HTTP API Agent Provider

- [ ] `HttpApiAgentProvider` implements `AgentProvider` by composing `Client` + `ProviderProfile` + `ToolRegistry` + `ExecutionEnvironment`
- [ ] Tool loop logic extracted from `Session.submit_single()` without behavioral changes
- [ ] All existing forge-agent tests pass without modification (behavioral equivalence)
- [ ] Loop detection, round limits, turn limits, and context warnings preserved

### 9.4 Session Refactoring

- [ ] `Session.submit_single()` delegates to `AgentProvider.run_to_completion()`
- [ ] `AssistantTurn` extended with `provider_tool_call_count`
- [ ] `SubmitResult` populated correctly for both provider types
- [ ] Event emission works for both provider types
- [ ] Abort handling works for both provider types
- [ ] Existing tests pass unchanged

### 9.5 Provider Configuration

- [ ] `ProviderConfig`, `ProviderEntry`, `ProviderKind` types defined
- [ ] CLI host (`forge-cli`) constructs providers from configuration
- [ ] No auto-discovery: providers are explicitly configured
- [ ] `--backend` flag accepts provider names

### 9.6 Integration

- [ ] End-to-end: pipeline runs with `HttpApiAgentProvider` (existing behavior, `--backend agent`)
- [ ] End-to-end: pipeline runs with `ClaudeCodeAgentProvider` (new behavior, `--backend claude-code`)
- [ ] Attractor codergen nodes work transparently with both provider types
- [ ] `cargo test` passes across all workspace crates
