# P150 — Scalable MCP Discovery and Tool Search

**Status**

- Implemented 2026-09-02 across management discovery, transcript expansion,
  native MCP search/detail retrieval, projection, and tests.
- Proposed and reviewed 2026-09-02 after validating Notion's 41-tool MCP
  server on hz01. Dropped: management pagination and a
  tool-detail method, truncation metadata, search-result tiers, aggregate
  context budgets, configurable limits, telemetry. Kept: one rendering rule
  for every search hit, byte-paged results, and a `names` mode that returns
  full definitions.
- Builds on [P143](p143-mcp-tool-discovery.md) and
  [P145](p145-native-mcp-execution.md). Supersedes P143's management view
  truncation and P145's search-result shape ("full input schemas for a
  bounded top-K; browse returns names and one-line descriptions"). The
  discovery limits themselves stay as documented in P143.

## Problem

The Notion validation exposed three behaviors that look like one failure:

1. **Management discovery truncates each title and description at 4 KiB.**
   That is not the discoverer's 16 KiB text bound. It is the gateway view
   projection in `mcp_api.rs`, which shares a 512 KiB text budget across the
   inventory under a static 4 KiB per-field cap so the response stays below
   the gateway's global 2 MiB JSON-RPC response guard. Four Notion
   descriptions were shortened, one from 9,172 bytes.
2. **The transcript shows only the first 4 KiB of each context entry.** The
   API already reports `textTruncated` and `contentRef`, and `blobs/read` is
   exempt from the response guard, but the web UI never reads
   `textTruncated`. A bounded preview therefore reads as data loss.
3. **`mcp_find_tools` over-delivers.** It returns 20 hits per page with full
   descriptions and, for a query, the full input schema of every hit. The
   result is stored as model-visible text without the 64 KiB model-visible
   cap that inline tools get. Raising the discovery bounds to 16 KiB text and
   512 KiB schema moved the worst case for one search result from roughly
   1.3 MiB to roughly 10 MiB. The scorer qualifies a tool on any single
   matched term, so a broad query against long descriptions matches most of
   an inventory.

The first two are presentation problems with truncation in the wrong layer.
The third is a context-shape problem: what lands in the model's context on
every search, before it has chosen anything. Larger limits fix the first and
make the third worse.

## Decision

Three small changes. No new API methods, no pagination of management
discovery, no truncation metadata, no per-deployment configuration.

### Management discovery: exempt, do not truncate

- Add the discovery method to the gateway's response-budget exemption beside
  `blobs/read`. Discovery is a deliberate bulk transfer that the discoverer
  already bounds with its decoded-inventory limit (16 MiB, typed failure).
  The 2 MiB guard exists for accidentally unbounded documents and does not
  apply.
- Delete the view-level text budget and per-field cap in
  `mcp_tool_discovery_success` together with the test that asserts them. The
  view returns whatever the discoverer retained; the discoverer's own 16 KiB
  UTF-8 truncation with an ellipsis stays.
- The web UI keeps its client-side filter and scroll box. It may clamp
  description lines visually. That is a UI choice and needs nothing from the
  backend.

Fallback, only if a 16 MiB management response proves unacceptable: keep the
guard, raise the per-field cap to the discoverer's 16 KiB bound and the shared
budget to about 1.5 MiB. That leaves inventories up to roughly 48 tools fully
intact and degrades larger ones visibly. Not the preferred path.

### Session transcript: UI only

- The web UI renders entries with `textTruncated` as the inline text, a
  truncated marker, and an expand action that reads the complete body through
  `blobs/read` by `contentRef`.
- No DTO change. Session content is never truncated at the source, so there
  is nothing to distinguish the preview from.

### Model retrieval: one hit shape, byte-paged, plus `names`

Contract. The meta-tool schema and description live in `tools::definitions`; the
description must explain the three modes to the model.

```text
mcp_find_tools { server?, query?, names?, cursor? }

browse  (no query, no names)  every allowlisted tool, ordered by server, name
search  (query)               ranked by the existing scorer; argument names
                              are indexed
detail  (server + names)      up to five tools, full definition, no window
```

Every browse or search hit has one shape:

```json
{
  "server": "notion",
  "name": "update-page",
  "description": "...",
  "inputSchema": { "...": "..." },
  "annotations": { "...": "..." },
  "truncated": "Call mcp_find_tools with server and names for the full definition."
}
```

- A hit is rendered whole when its serialized form is at or under 8 KiB.
  Otherwise the description is cut first, on a UTF-8 boundary, until the hit
  fits. If the input schema alone does not fit, it is replaced by the list of
  top-level argument names. `truncated` carries the note above and is absent
  on whole hits.
- Pages fill in rank order until 64 KiB of serialized hits, always at least
  one hit, then return `nextCursor`. The cursor stays an integer offset.
- `names` requires `server`, accepts at most five names, and returns full
  definitions with no window. A tool with a 512 KiB schema is the operator's
  choice, paid once for a tool the model already selected.
- The scorer keeps its deterministic ranking. Top-level input-schema property
  names join the name-side term set with a small penalty, so queries such as
  `page id` or `database` find tools through their arguments. The single-term
  qualification rule stays: a loose hit now costs at most one window, and
  name matches already sort ahead of description matches.
- `mcp_call` argument validation includes the tool's input schema in its
  error text. A model working from a truncated hit recovers with one retry
  instead of a detail call.
- Hits and pages are serialized compactly, and the window and page budget
  are measured on that compact form. Today's pretty-printed visible text
  inflates Configurator hits by a third at the median and nearly doubles the
  largest schemas, all of it whitespace the model pays for.
- Three hard-coded constants: 8 KiB per hit, 64 KiB per page, five names per
  detail call.

Calibration, measured on the Configurator inventory of 102 tools:

| field | median | p90 | max |
|---|---|---|---|
| description | 126 B | 198 B | 292 B |
| input schema | 247 B | 2.0 KiB | 16.7 KiB |
| both | 379 B | 2.2 KiB | 16.9 KiB |

As compact hits with server, name, and annotations included, 95 of 102 fit
whole under 8 KiB; the rest are session and profile schemas that inline
definition trees. Notion is the mirror image, with long
descriptions and moderate schemas. Its four descriptions above 4 KiB are the
expected truncation cases.

## Acceptance

- Live Notion discovery on the management page shows all 41 tools with every
  description intact.
- A transcript entry longer than 4 KiB renders a truncated marker and expands
  to the exact CAS body.
- A `mcp_find_tools` search against Notion returns a page at or under 64 KiB
  of hits. Whole hits carry full schemas, oversized hits carry the truncation
  note, and a detail call with `names` returns the full definition.
- A query for an argument name finds the tools that declare it.
- Browsing a search-exposure server pages to completion, and the cursor never
  loops.
- The gateway still rejects a non-exempt response above 2 MiB; the discovery
  method is exempt.

## Non-Goals

- Preloading inventories because a model has a large context window.
- Pagination or a tool-detail method for management discovery.
- Truncation metadata such as `originalBytes` on management views.
- Per-deployment configuration of discovery or search limits.
- Changing the discovery limits documented in P143.
- Persisting a management discovery snapshot as execution authority.

## Implementation Slices

### Slice 1 — Management discovery and transcript

- `gateway/http.rs`: add the discovery method to `response_budget_exempt`.
- `gateway/service/mcp_api.rs`: remove the `DISCOVERY_VIEW_*` constants and
  the truncation in `mcp_tool_discovery_success`; drop the response-budget
  projection test. The discoverer's truncation and its test stay.
- `platform/web`: truncated transcript entries expand through `blobs/read`.
- P143 Limits section: drop the sentence about the management projection
  sharing 512 KiB of text.

### Slice 2 — Search result shape

- `worker/mcp.rs` `find_tools`: uniform hit rendering with the 8 KiB window,
  byte-paged results, `names` mode, compact serialization of the visible
  text; `validate_search_call` error text carries the schema.
- `crates/mcp/src/search.rs`: argument-name terms.
- `gateway/service/mcp_api.rs`: the meta-tool schema gains `names`; the
  description explains browse, search, and detail.
- `api-projection`: the transcript display for a detail call names the
  requested tools.
- `tests/mcp_live.rs`: update result-shape and pagination assertions; add a
  detail-mode case and an oversized-hit case against a synthetic fixture.
- P145 Search Exposure section: point at this document for the result shape.
