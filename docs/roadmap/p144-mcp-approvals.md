# P144 — MCP Tool-Call Approvals

**Status**

- Proposed 2026-08-31. Supersedes the sketch in
  `later/pNNN-mcp-approval-flow.md` (production incident 2026-07-10: an OpenAI
  `mcp_approval_request` became a run's terminal output).
- Builds on P67 (provider-hosted remote MCP and opaque MCP context entries),
  P143 (the MCP server record owns `approvalDefault`), P129 (live run-control
  admissions), and P92 (parked runs). The native-execution half lands with
  [P145](p145-native-mcp-execution.md).
- One approval surface, two backends: provider-hosted MCP approvals
  (OpenAI Responses approval request/response items) and native MCP approvals
  (Lightspeed holds the tool dispatch). Clients see the same pending-approval
  model either way.

## Problem

`approvalDefault` exists on every MCP server record, but nothing in the product
can answer an approval:

1. **Provider mode, OpenAI Responses.** `require_approval: "always"` is lowered
   into the request, and the returned `mcp_approval_request` item is preserved
   only as a `ProviderOpaque` context entry. Nothing ever constructs an
   `mcp_approval_response`, so the call can never proceed.
2. **Terminal-output corruption.** When the provider reports `finish: stop`
   with no assistant message, the engine's `final_output_ref` fallback selects
   the last context entry — which in this situation is the approval-request
   JSON. The run completes "successfully" with an approval request as its
   final output, and the web transcript (which hides opaque provider entries)
   shows nothing at all.
3. **Provider mode, Anthropic.** The Anthropic MCP connector has no approval
   protocol; `RemoteMcpApprovalPolicy::Always` is a hard materialization error
   (`anthropic_messages.rs`). That refusal is correct but leaves `always`
   unusable on Anthropic sessions until native execution exists.
4. **Native mode (P145).** Once Lightspeed executes MCP calls itself, approval
   becomes entirely Lightspeed's responsibility; there is no provider to defer
   to.

Until P144, `approval: always` is unusable and the documented guidance is to
configure `never`. That inverts the safe default for exactly the servers that
most need a human gate.

## Decision

Model "a tool call is waiting for a human decision" as first-class session
state with one client-facing shape, regardless of which side executes the call:

- A run with undecided approvals is **parked**, like a run waiting on an
  await. It is not complete, not failed, and holds no activity slot.
- Each pending approval is identified by a session-counter id
  (`approval_<n>`, P138 style — never the provider's opaque request id, which
  stays internal on the engine fact).
- Decisions are run-control admissions: they land on a live session the same
  way steer/cancel do, and `session/runs/cancel` rejects everything still
  pending as part of cancelling the run.
- The MCP server record's `approvalDefault` remains the only policy authority
  (P143). Sessions, profiles, tool annotations, and models cannot grant,
  narrow, or bypass approval. `readOnlyHint`/`destructiveHint` may inform UI
  badges, never the decision to gate.

### Approval lifecycle

```text
requested ──► approved ──► executed (provider continuation or native dispatch)
        └───► rejected ──► rejection result visible to the model
        └───► cancelled (run cancel resolves all pending as rejected)
```

An approval is decided exactly once. A run continues only after every pending
approval in it is decided; partial decisions update state but do not wake the
run.

## Engine Model

### Typed approval facts

Stop treating approval requests as purely opaque. On the generation response
path, parse the minimal deterministic facts alongside the preserved opaque
entry:

```rust
pub struct McpApprovalRequested {
    pub approval_id: ApprovalId,          // approval_<n>, session counter
    pub origin: McpApprovalOrigin,        // Provider { provider_request_id } | Native { call_id }
    pub server_id: String,
    pub server_label: String,
    pub tool_name: String,
    pub arguments_ref: BlobRef,
}

pub enum McpApprovalDecision { Approved, Rejected }
```

This follows the existing rule: parse only the reducer facts needed for
deterministic branching; the raw provider item stays blob-backed. The decision
event records the decision, an optional bounded note, and the deciding
principal when the gateway has caller identity (P90).

### Run parking

A turn that ends with undecided approvals does not complete the run and does
not plan the next turn. The run parks with a pending-approvals reason (exact
run-state spelling decided with the P92 vocabulary at implementation). P129
semantics extend naturally:

- steering is accepted while parked and materializes at the next turn;
- a second `session/runs/start` queues;
- `session/runs/cancel` rejects all pending approvals and cancels the run;
- decisions are the only admissions that can unpark the run.

### Terminal-output correctness (independent fix)

Regardless of the rest of P144, fix the fallback now:

1. `final_output_ref` selection must never choose a `ProviderOpaque` context
   entry.
2. A successful generation with no assistant message either parks the run
   (undecided approvals exist) or finishes with `output_ref: None`. It must
   never surface provider bookkeeping JSON as user-visible output.
3. Regression test: an OpenAI Responses fixture containing
   `mcp_approval_request` and no `message` must not produce a completed run
   whose output ref points at the approval request.

This slice is safe to land before any approval UX exists.

## Provider Backend (OpenAI Responses)

When the response contains `mcp_approval_request` items:

1. Preserve the opaque entry exactly as today (context replay needs it).
2. Emit one `McpApprovalRequested` fact per item, mapping `server_label` back
   to the session's RemoteMcp spec for `server_id`.
3. If the same response also queued ordinary tool calls, run that tool batch
   first; park at the boundary where the next turn would otherwise be planned.
4. On full decision, plan the next turn. The adapter lowers each decision into
   the provider's continuation input:

```json
{ "type": "mcp_approval_response", "approval_request_id": "mcpr_…", "approve": true }
```

The decision travels as a typed context entry (append-only, so the provider
prefix cache holds; P137). The provider then executes approved calls and
returns ordinary `mcp_call` items; rejected calls are the provider's rejection
to explain to the model.

Anthropic provider mode is unchanged: `always` remains a hard error with a
message pointing at native execution or an OpenAI Responses model. Do not
emulate an approval loop the provider does not have.

## Native Backend (with P145)

Native execution puts the gate exactly where it belongs — before dispatch:

1. The engine plans the tool batch as usual. The gate is evaluated at the
   **dispatch boundary**: the runtime resolves the call's spec and effective
   policy, and a call that requires approval is reported back as
   needs-approval instead of executing; the engine emits an
   `McpApprovalRequested` fact with `origin: Native { call_id }` and the
   call enters `awaiting_approval`. One gate location covers both P145
   exposures — an injected call's server is knowable from its namespaced
   name, but a search-exposure `mcp_call` carries server and tool in
   arguments, which reducers do not parse. The fact always carries the
   resolved server and tool, so approval cards and transcripts never render
   a generic `mcp_call` row.
2. Approved calls dispatch through the ordinary per-call activity path.
3. Rejected calls complete immediately with a deterministic rejection result
   (`{"approved": false, "reason": …}` as a tool error), so the model sees the
   refusal and can adapt instead of stalling.
4. Calls not requiring approval in the same batch execute without waiting;
   the batch completes when every call has a result.

Effective policy in native mode: `always` and `never` mean what they say;
`providerDefault` resolves to **`always`** — there is no provider default to
defer to, matching OpenAI's own safe default. Unattended automation opts into
`never` explicitly on the server record.

## API

Names are illustrative; the exported contract decides the generated spelling.

```text
session/runs/approvals/decide
```

```rust
pub struct RunApprovalsDecideParams {
    pub session_id: String,
    pub decisions: Vec<ApprovalDecisionInput>,
}

pub struct ApprovalDecisionInput {
    pub approval_id: String,
    pub decision: ApprovalDecisionKind,   // approve | reject
    pub note: Option<String>,             // bounded, audit/display only
}
```

- `RunView` gains `pendingApprovals`: approval id, server id, tool name, a
  bounded arguments preview, requested-at, and origin. Full arguments stay
  blob-backed and readable through the existing item/blob paths.
- Deciding an unknown, already-decided, or other-run approval is a typed API
  error; valid decisions in the same request still apply (per-decision
  outcome in the response).
- A notification/event on `session/events/read` announces newly pending
  approvals so clients do not poll `session/read`.

### Clients

- Web session view: an approval card per pending approval (server, tool,
  arguments, annotation badges as hints) with Approve/Reject; the run header
  shows the parked-awaiting-approval state.
- CLI chat: print pending approvals and accept an approve/reject input.
- Bot sessions: a parked run simply holds its lane; the bot event `outcome`
  stays pending until the run resolves. Surfacing approvals in the bot
  console/chat is a follow-up; v1 makes the state visible through the
  ordinary session API the console already reads.

## Timeouts

v1 has no automatic approval deadline: a pending approval parks the run until
a human decides or the run is cancelled. Cancellation is the escape hatch and
must resolve every pending approval as rejected on its way down. A per-server
`approvalTimeout` (auto-reject after a bound, primarily for unattended bots)
is a follow-up once real demand shows the right default; do not add a silent
auto-approve under any name.

## Implementation Slices

### Slice 1 — Terminal-output fix

- Exclude `ProviderOpaque` entries from `final_output_ref` fallback; stop
  finishes with no assistant message resolve to `output_ref: None`.
- Regression fixtures for OpenAI approval-request-only responses.

### Slice 2 — Approval facts and parked runs

- `ApprovalId` counter, `McpApprovalRequested`/decision events, reducer state,
  parked-run behavior, cancel-resolves-pending.
- OpenAI Responses parsing of `mcp_approval_request` into facts alongside the
  opaque entry.

### Slice 3 — Decide API and provider continuation

- `session/runs/approvals/decide`, projection of `pendingApprovals`,
  notifications, per-decision outcomes.
- Typed decision context entry and OpenAI `mcp_approval_response` lowering;
  live test: `always` server → park → approve → executed call; reject →
  provider rejection surfaced.

### Slice 4 — Clients

- Web approval cards + parked state, CLI prompt flow.

### Slice 5 — Native gate (with P145)

- `awaiting_approval` call state, gated dispatch, deterministic rejection
  results, `providerDefault`→`always` resolution in native mode.

## Tests

- Engine: approval facts are deterministic and replay-stable; a run with
  undecided approvals never completes; decisions are single-shot; cancel
  rejects pending; steering while parked lands at the next boundary.
- Adapter: approval-request parsing, decision lowering, prefix stability of
  the appended decision entry.
- API: decide validation (unknown/duplicate/foreign ids), projection shape,
  notification emission.
- Native (with P145): mixed batch of gated and ungated calls; reject produces
  a model-visible tool error, not a run failure.
- Regression: the 2026-07-10 incident shape end to end.

## Acceptance

1. `approval: always` is usable end to end on OpenAI Responses provider mode
   and on native execution; a human can approve or reject from the web UI and
   CLI, and the run continues correctly either way.
2. No run ever reports an approval request (or any opaque provider entry) as
   its final output.
3. Pending approvals are visible on `RunView` with stable counter ids;
   decisions are auditable events carrying the deciding principal when
   available.
4. Cancelling a run with pending approvals rejects them and cancels cleanly.
5. Anthropic provider mode still refuses `always` with an actionable message.

## Non-Goals

- Auto-approval from tool annotations, model output, or any heuristic.
- Per-session or per-profile approval overrides (P143 removed them).
- Emulating an approval protocol on providers that lack one.
- Approval deadlines/expiry policies in v1.
- A standing approvals inbox across sessions; v1 is per-run state.

## Follow-ups

- Bot console surfacing and channel notification of pending approvals.
- Per-server `approvalTimeout` with auto-reject.
- Per-tool approval policy on the server record if All/Selected-style
  granularity proves insufficient.
