# P148 — Scalable MCP Discovery and Tool Search

**Status**

- Proposed 2026-09-02 after validating Notion's 41-tool MCP server on hz01.
- Builds on [P143](p143-mcp-tool-discovery.md) and
  [P145](p145-native-mcp-execution.md). It keeps bounded discovery and native
  search exposure, but supersedes their sizing, projection, and search-result
  policies where large inventories are silently shortened or over-delivered.

## Problem

The Notion validation exposed three different behaviors that look like one
failure:

- management discovery really truncates each title or description at 4 KiB;
  four Notion descriptions were shortened, including one from 9,172 to 4,096
  bytes;
- session event projection shows only the first 4 KiB of an entire tool result,
  even though the complete CAS blob remains available and is sent to the model;
- `mcp_find_tools` returns 20 full descriptions per page and, for a query, also
  returns every matching input schema without an aggregate result budget.

The first loses useful operator-visible metadata, the second misleadingly
looks like data loss, and the third can waste context or produce a very large
result when schemas approach their individual limits. The current 16 KiB
description and 2,048-tool discovery bounds did not affect Notion, but they are
implementation policy rather than MCP protocol limits and are too narrow for
the enterprise servers this path must support.

Large model context windows do not make eager inventory injection desirable.
They should let a selected tool retain its complete definition, while search
keeps unrelated definitions out of the prompt.

## Decision

Separate four concerns instead of using one set of truncation constants:

1. **Discovery safety envelope.** Proposed defaults are keep (even reduce) to 1024 tools, 256
   pages, 64 MiB of decoded inventory, 128 KiB per title/description, 2 MiB per
   schema, and JSON depth 64. These remain configurable with a larger absolute
   deployment ceiling. Crossing a completeness bound returns a typed failure;
   shortening optional text must set `truncated` and `originalBytes` rather
   than silently appending an ellipsis.
2. **Management projection.** Make discovery results cursor-paginated. List
   pages return compact summaries and explicit truncation metadata; a tool
   detail method returns the retained description, annotations, and schemas.
   The UI loads detail on expansion instead of forcing the whole inventory
   through one gateway response.
3. **Model retrieval.** Keep only `mcp_find_tools` and `mcp_call` in context for
   search-exposure servers. Search ranks names, titles, descriptions, argument
   names, and argument descriptions; it returns five results by default and at
   most twenty. Browse returns summaries. A detail operation returns the full
   definition for one or a few selected tools. Search pages have a 256 KiB
   serialized hard budget and always return a cursor when matches remain.
4. **Session presentation.** Keep the small inline event preview, but label it
   with displayed and total bytes and let the client expand or download the
   existing `contentRef`. Preview truncation and source truncation must be
   visually distinct.

Limits are enforced in bytes for transport and memory safety. Model adapters
also account for tokens and may request fewer results, but they must not
silently rewrite a selected tool definition. Telemetry records inventory size,
largest fields, pages, search-result bytes, and each applied bound without
recording tool metadata or credentials.

## Acceptance

- The live Notion inventory shows all 41 tools and all four previously shortened
  descriptions through tool detail.
- A transcript initially renders a bounded preview and can retrieve the exact
  complete search result from its CAS reference.
- Synthetic 10,000-tool and maximum-schema fixtures remain within declared
  memory, response, and context budgets; pagination cannot loop indefinitely.
- A natural-language query finds tools from argument metadata and returns a
  small ranked set; obtaining a full selected definition never requires
  browsing the entire inventory.
- Every refusal or partial field identifies the specific bound that applied.

## Non-Goals

- Preloading every advertised tool because a model has a large context window.
- Persisting a management discovery snapshot as execution authority.
- Accepting unbounded responses or inventories; a 1 GiB catalog remains a hard
  failure.
