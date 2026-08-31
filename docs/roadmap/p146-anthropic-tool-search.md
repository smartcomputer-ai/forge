# P146 — Anthropic Tool Search for Provider-Mode MCP

**Status**

- **Implemented 2026-08-31.** The Anthropic adapter now uses the current
  `mcp-client-2025-11-20` connector shape (`mcp_servers` plus one
  `mcp_toolset` per server), translates record allowlists into toolset
  configuration, honors `deferLoadingDefault` through one BM25 Tool Search
  Tool, rejects known pre-4.5 Claude models, and places prompt-cache markers
  only on non-deferred tools. The public MCP live suite exercises search and
  the resulting MCP call end to end.
- Builds on P67 (Anthropic MCP lowering), P143 (record-owned
  `deferLoadingDefault`), and P137 (breakpoint placement). Independent of
  P145, which does not touch provider-mode lowering.

## Problem

`deferLoadingDefault` is a record-owned policy (P143), but only the OpenAI
Responses adapter honors it (`defer_loading: true` on the lowered
`type: "mcp"` entry — OpenAI then withholds remote tool schemas until
needed). The Anthropic adapter silently dropped the field because the
Anthropic MCP connector historically had no equivalent — the same silent
asymmetry that still drops `server_description`.

Anthropic now ships the Tool Search Tool (GA, no beta header): tools marked
deferred are excluded from the context prefix, a server-side search tool
discovers them on demand, and the API expands `tool_reference` blocks
inline — explicitly designed to preserve the cached prefix. MCP connector
integration is first-class: deferral is set once on the `mcp_toolset`
entry's `default_config` for the whole server (or per tool via `configs`),
which matches Lightspeed's record-level field exactly.

## Decision

Provider-mode Anthropic lowering uses the current connector contract for every
server: `mcp_servers` contains connection and authentication data, while a
matching `mcp_toolset` in `tools` carries selection and loading policy. Every
server has exactly one toolset. An authored allowlist lowers to
`default_config.enabled: false` plus one `configs.<name>.enabled: true` entry
per allowed tool; inventories remain live at the provider and are never
snapshotted by Lightspeed.

When a provider-mode Anthropic request carries at least one RemoteMcp spec
with `defer_loading: true`, `materialize` additionally emits:

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

Requests with no deferred MCP server emit their ordinary non-deferred
`mcp_toolset` entries and no search tool. Flipping `deferLoadingDefault` is a
config change and takes the one-time cache invalidation any tools-array change
costs.

## Constraints the adapter must enforce

- **Never defer the search tool itself**, and at least one tool in `tools`
  must be non-deferred — the search tool satisfies this in the pure-MCP
  session case (Anthropic's documented "normally the tool search tool
  itself").
- **P137 breakpoint interplay:** a deferred tool cannot carry
  `cache_control` (the API rejects it). The Anthropic adapter places its tool
  breakpoint on the last **non-deferred** tool. Tool-search expansion itself
  is prefix-preserving, so P137's cache economics survive. The prompt-caching
  rule in `AGENTS.md` and the breakpoint layout in
  [P137](p137-prompt-caching.md) match the defer-aware adapter behavior.
- **Model gating:** tool search is supported on the 4.5 family and later.
  A deferred provider-mode server on a recognized older Anthropic model is a
  typed materialization error naming the constraint, consistent with the
  existing `approval: always` hard error — never a silent drop. Unknown model
  naming schemes are left to the provider so future and Anthropic-compatible
  endpoints are not rejected speculatively.
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
- Live (`anthropic_messages_mcp_live`): a deferred public MCP server
  round-trips through a BM25 `server_tool_use`, a
  `tool_search_tool_result`, and the resulting MCP tool call. General prompt
  cache retention remains covered by the dedicated Anthropic caching live
  suite; this small public MCP fixture is below Anthropic's cacheable-token
  floor.

## Non-Goals

- Per-tool deferral (`configs`) or exposing the search variant in the
  record/API.
- Tool search over Lightspeed function tools or native-mode tools.
- OpenAI-side changes; `defer_loading` there already works.
- Emulating deferral on providers without it.
