# P146 — Anthropic Tool Search for Provider-Mode MCP

**Status**

- Proposed 2026-08-31. Small, self-contained adapter change: honor the MCP
  server record's `deferLoadingDefault` on Anthropic provider-mode sessions
  through Anthropic's Tool Search Tool, restoring parity with the OpenAI
  Responses adapter.
- Builds on P67 (Anthropic MCP lowering), P143 (record-owned
  `deferLoadingDefault`), and P137 (breakpoint placement). Independent of
  P145, which does not touch provider-mode lowering.

## Problem

`deferLoadingDefault` is a record-owned policy (P143), but only the OpenAI
Responses adapter honors it (`defer_loading: true` on the lowered
`type: "mcp"` entry — OpenAI then withholds remote tool schemas until
needed). The Anthropic adapter silently drops the field because the
Anthropic MCP connector historically had no equivalent — the same silent
asymmetry that already drops `server_description`.

Anthropic now ships the Tool Search Tool (GA, no beta header): tools marked
deferred are excluded from the context prefix, a server-side search tool
discovers them on demand, and the API expands `tool_reference` blocks
inline — explicitly designed to preserve the cached prefix. MCP connector
integration is first-class: deferral is set once on the `mcp_toolset`
entry's `default_config` for the whole server (or per tool via `configs`),
which matches Lightspeed's record-level field exactly.

## Decision

When a provider-mode Anthropic request carries at least one RemoteMcp spec
with `defer_loading: true`, `materialize` emits:

1. one tool search tool entry, added once at a deterministic, stable
   position in the lowered `tools` array:

```json
{ "type": "tool_search_tool_bm25_20251119", "name": "tool_search_tool_bm25" }
```

2. `default_config: { "defer_loading": true }` on each deferred server's
   `mcp_toolset` entry. Whole-server granularity only — the record field is
   server-level, and the authored allowlist already narrows the tool set.
   Do not add per-tool `configs` deferral.

The BM25 variant is the default (natural-language queries; the regex
variant's Python-pattern syntax is a needless failure mode for tool
discovery). The variant string is versioned and model-gated; keep it in the
adapter beside the other Anthropic type constants, not in engine state or
the record.

Requests with no deferred MCP server are unchanged — no search tool
appears. Flipping `deferLoadingDefault` is a config change and takes the
one-time cache invalidation any tools-array change costs.

## Constraints the adapter must enforce

- **Never defer the search tool itself**, and at least one tool in `tools`
  must be non-deferred — the search tool satisfies this in the pure-MCP
  session case (Anthropic's documented "normally the tool search tool
  itself").
- **P137 breakpoint interplay:** a deferred tool cannot carry
  `cache_control` (the API rejects it). The Anthropic adapter's "last tool"
  breakpoint must be placed on the last **non-deferred** tool; add the
  placement rule and a lowering test. Tool-search expansion itself is
  prefix-preserving, so P137's cache economics survive. At implementation,
  update the documented placement rule from "last tool" to "last
  non-deferred tool" wherever it is stated — the prompt-caching bullet in
  `AGENTS.md` and the breakpoint layout in
  [P137](p137-prompt-caching.md) — so the docs keep matching the adapter.
- **Model gating:** tool search is supported on the 4.5 family and later.
  A deferred provider-mode server on an older Anthropic model is a typed
  materialization error naming the constraint, consistent with the existing
  `approval: always` hard error — never a silent drop (the silent drop is
  the bug this doc removes).
- The MCP connector beta header remains provider client/runtime
  configuration (P67 rule); tool search itself needs no beta.

## Scope Guard

Provider mode only. Native execution (P145) deliberately behaves
identically on every API kind; its answer to large inventories is
`exposure: search`, and it must not grow a provider-specific tool-search
variant. This doc is parity for the passthrough path, not a third exposure.

## Tests

- Materialization: deferred server produces the search tool entry plus
  `default_config.defer_loading` on its `mcp_toolset`; two deferred servers
  produce one search tool; a non-deferred server alongside them is
  untouched; no deferred server produces no search tool.
- Breakpoint: `cache_control` lands on the last non-deferred tool when the
  final tools are deferred toolsets.
- Model gating: unsupported model + deferred server is a typed error.
- Redacted request blobs carry the new entries; auth injection by
  `server_label`/`name` is unaffected.
- Live (best-effort, existing `anthropic_messages_live` / `*_mcp_live`
  style): a deferred public MCP server round-trips — search discovery,
  `tool_reference` expansion, and a tool call — with cache reads holding
  across turns.

## Non-Goals

- Per-tool deferral (`configs`) or exposing the search variant in the
  record/API.
- Tool search over Lightspeed function tools or native-mode tools.
- OpenAI-side changes; `defer_loading` there already works.
- Emulating deferral on providers without it.
