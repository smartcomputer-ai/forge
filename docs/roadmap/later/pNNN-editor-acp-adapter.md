# PNNN: Editor ACP Adapter (Agent Client Protocol)

**Status**
- Later / exploratory. Written 2026-08-18.
- Targets Zed's **Agent Client Protocol** (`agentclientprotocol.com`): a
  JSON-RPC 2.0 protocol between code editors (clients) and coding agents.
  Not to be confused with the retired IBM "Agent Communication Protocol",
  which was folded into A2A; agent-to-agent interop is covered by
  [pNNN-a2a-protocol-adapter.md](pNNN-a2a-protocol-adapter.md). The two
  adapters are orthogonal and share only the admission rules.
- References:
  - Introduction: <https://agentclientprotocol.com/get-started/introduction>
  - Protocol overview: <https://agentclientprotocol.com/protocol/overview>

## Goal

Make a hosted Lightspeed session usable from any ACP-capable editor (Zed,
JetBrains, Neovim, Emacs, …) through a thin adapter, without changing the
internal execution model and without letting the editor's local filesystem
become a second, implicit environment.

## What editor ACP is

Two roles, both speaking JSON-RPC over stdio (HTTP/WebSocket for remote
agents is still in development upstream):

**Agent methods** (the editor calls the agent):

```text
initialize            version + capability negotiation
authenticate          optional
session/new           create a conversation (cwd, optional mcpServers)
session/load          resume an existing session (capability-gated)
session/prompt        send a user turn; returns when the turn ends (stopReason)
session/cancel        notification
session/set_mode      switch operating mode
```

**Client methods** (the agent calls the editor):

```text
session/update            notification: message chunks, tool calls/updates
                          (with diffs and locations), plans, mode changes
session/request_permission  ask the user to approve a tool call
fs/read_text_file, fs/write_text_file      optional client capability
terminal/create|output|wait_for_exit|kill|release   optional client capability
elicitation/create, elicitation/complete   optional structured input
```

Content parts reuse MCP's JSON representations.

## Fit

The agent-side surface is a client-facing adapter over `api`, exactly like
the CLI. The client-side surface is where ACP touches Lightspeed's
environment model, and that is where the design decisions are.

### Agent-side mapping (straightforward)

```text
initialize / authenticate  -> gateway API key (P90); advertise capabilities
                              (loadSession: yes; promptCapabilities from profile)
session/new                -> session/start, or session/managed/start with a
                              profile selected by adapter configuration
session/load               -> session/read + session/events/read replay,
                              emitted as session/update history
session/prompt             -> session/runs/start (acceptance boundary), then
                              follow session/events/read until the run is
                              terminal; map terminal state to stopReason
                              (end_turn | cancelled | refusal | max_tokens …)
session/cancel             -> session/runs/cancel -> CancelRun
session/set_mode           -> session/config/put with expected revision
                              (modes are adapter-defined config presets)
session/update (agent)     -> projection of api notifications: assistant text
                              chunks, tool_call / tool_call_update (status,
                              content, diffs, locations), plan entries
```

Rules carried over from the A2A adapter apply unchanged: all inbound goes
through `RequestRun` / `SubmitMessage` / `CancelRun`; the validated wake
(`ResumeToolBatch`, P94's "ResumeAwait") is never exposed; runs stay
immutable; states collapse conservatively; idempotency keys are mapped.

`session/prompt` blocking until the turn ends is compatible with treating
`session/runs/start` as an acceptance boundary: the adapter accepts, then
streams events and returns the stop reason when the run is terminal.

`session/new` may carry `mcpServers` from the editor. Under P110, MCP
authentication belongs to universe MCP records and sessions select only
`serverId`, so the adapter must not pass editor-supplied MCP credentials
through. It either rejects them, or maps them onto pre-registered universe
records by name. Client-supplied stdio MCP servers cannot be reached from a
hosted worker at all.

### Permission and elicitation need an opt-in await wake (mailbox removed)

`session/request_permission` and `elicitation/create` are the same shape as
a parked `await` woken by an inbound reply. P134 slice 7 removed the fleet
mailbox (`await { mailbox }`); this adapter would reintroduce that wake as an
opt-in await field (P129's note). The run parks, the adapter projects the
pending question to the editor, the user's answer enters as `SubmitMessage`,
and the engine validates the wake. This dovetails with the
[P144 MCP approvals](../p144-mcp-approvals.md) parked-run model and needs no
new engine vocabulary. If the editor disconnects, the run stays parked; reconnecting via
`session/load` re-projects the pending permission.

### Client-side fs/terminal vs the environment protocol

ACP's optional `fs/*` and `terminal/*` client capabilities are *not* a
replacement for, or an overlay on, the environment protocol
(`crates/environment-protocol`, `lightspeed-envd`). They differ in every
axis that matters:

| | Environment protocol | ACP client fs/terminal |
|---|---|---|
| Host | VM/container daemon, provider-managed or external | the editor process on the user's machine |
| Caller | Temporal tool activities, durable, retried, `EnvironmentNotReady` / wake-on-use (P114/P125/P126) | the agent, interactively, only while the editor is connected |
| Surface | `fs/readFile` (ranged), `writeFile`, `readDirectory`, `createDirectory`, `copy`, `remove`, `getMetadata`, `globFiles`, `searchText`; `process/start|read|write|resize|terminate`, jobs, idle, control plane (`controller/*`, power, ingress) | text read/write; five terminal verbs; permission prompts |
| Trust | universe-scoped credentials and provider bindings | whatever the editor allows |

The environment protocol is the more general one; ACP's could at most be a
degraded profile of it. Three options, in order of preference:

1. **Default: hosted tools stay in Lightspeed environments; the editor sees
   only the conversation.** The adapter does not require `fs`/`terminal`
   client capabilities. Tool calls executed against the session's active
   environment (or VFS) are projected to `session/update` tool_call entries
   with diffs and locations, so the editor renders them. Zero architectural
   risk and covers "use Lightspeed from my editor".

2. **Optional: an "editor environment" provider.** The adapter binary
   registers itself as an *external* environment target whose data plane is
   the ACP client (`fs/readFile` → `fs/read_text_file`, `process/start` →
   `terminal/create`, …). Profiles select it through the ordinary
   `environment: existing` intent; P113's rule holds — no fusion, overlay,
   or implicit synchronization with VFS or other environments. Gaps that
   make it a *degraded* target: no `searchText`, `globFiles`,
   `readDirectory`, `copy`, `remove`, binary or ranged reads, `resize`; no
   durability across the editor process; no control plane. Requires the
   data plane to advertise per-target filesystem/process capabilities (as
   P126 does for `powerStates`) so tools fail cleanly rather than by
   surprise. Do this only if "agent edits the files open in my editor" is
   actually wanted; it is not a prerequisite for option 1.

3. **Rejected: adopt ACP fs/terminal as the environment data plane.**
   Strictly less capable, no durability, no control plane, remote transport
   unfinished upstream.

## Adapter shape

A small binary, `lightspeed-acp`, speaking ACP on stdin/stdout and talking
JSON-RPC to the gateway (reusing `crates/cli`'s client). Configuration:
gateway URL, API key, universe, default profile, and (if option 2 is ever
built) whether to register as an editor environment. Editors launch it as
their "agent" command.

## Open design questions

- Mode presets: which `session/config` documents are exposed as ACP modes,
  and are they adapter config or profile-defined?
- `session/load` history depth: replay everything, or the last N runs with a
  cursor?
- Where does the "current run" for `session/cancel` come from when a session
  has concurrent runs (fleet children)? Likely: cancel the prompt-origin run
  only; children follow structured cancellation.
- Is option 2 wanted at all, and if so, is it an environment provider crate or
  a mode of `lightspeed-acp`?
- Remote transport: wait for upstream HTTP/WS, or expose ACP over the
  gateway's existing WebSocket as a second frontend?

## Non-goals

- Do not make ACP the internal engine model or expose reducer internals.
- Do not accept editor-supplied MCP credentials into sessions (P110).
- Do not add a second public resume path; permission answers are
  `SubmitMessage`.
- Do not fuse editor-side files with VFS or provisioned environments (P113).
- Do not target the retired IBM Agent Communication Protocol.
