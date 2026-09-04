# P158 — Anthropic Hosted Web Tools

**Status**

- Implemented 2026-09-04 (uncommitted).

## Problem

The session contract already grants web search and fetch independently, but
toolset resolution was asymmetric: search was hard-wired to OpenAI Responses
and fetch was always the local guarded function. Anthropic Messages therefore
rejected the search grant and could not use Anthropic's hosted fetch tool.

Anthropic server tools also introduce two response behaviors that the generic
Messages adapter must preserve. A `pause_turn` response asks the client to
resend the assistant content unchanged and continue the same turn. Search and
fetch citations carry provider-only replay fields while clients need a small,
provider-neutral source view.

## Design

- The internal toolset config mirrors the public capability grant: one shared
  web search config plus a fetch flag. Resolution chooses provider-native or
  locally executed implementations for the selected API kind.
- OpenAI Responses retains hosted `web_search`; its fetch grant retains the
  guarded local `web_fetch` function. Anthropic Messages lowers search to
  `web_search_20250305` and fetch to `web_fetch_20250910`, both as
  provider-hosted native tools with bounded uses. Hosted fetch also caps
  returned content tokens and enables citations.
- Allowed and blocked domains flow from the session contract into both hosted
  search implementations. Anthropic admits only one list at a time, matching
  its native request contract.
- Anthropic `server_tool_use` and server-tool result blocks stay as exact raw
  provider context, in their original order. A run of consecutive text blocks
  becomes one assistant message whose text is their exact concatenation, so a
  cited answer is one message rather than one fragment per cited span. The
  preamble the model writes before a search stays its own message, and the
  search call projects as a tool step between the two.
- When a text run carries citations, the message is followed by one
  provider-opaque entry holding the run's exact block array, encrypted
  indexes included. The adapter replays those blocks in place of the neutral
  text whenever the entry immediately follows its message with the same
  source; without it (a truncated turn keeps only the text, or an entry was
  removed) the text replays as is. The core entry model is unchanged: no
  companion flag, no second content reference, and no citation data in
  engine state. API projection derives provider-neutral citations (URL,
  title, cited text) from the cited entry and attaches them to the preceding
  message, resolving fetch citations against the fetch results earlier in the
  same turn. Encrypted provider fields never become engine branching facts.
- OpenAI Responses `url_citation` annotations follow the same model: the exact
  cited message item follows its message and replays in its place, and
  projection derives the same citations from its annotations. The separately
  included search sources remain raw provider context rather than being
  mistaken for citations.
- The Anthropic runtime handles `pause_turn` inside one generation activity by
  appending each raw assistant response and calling Messages again. Usage and
  committed context are combined across continuations, and the loop is capped
  at eight provider pauses.

## Tests

- Tool builders cover Anthropic search/fetch JSON, domain validation, and
  provider-hosted execution.
- Toolset and session-admission tests cover both Anthropic grants and preserve
  the OpenAI behavior.
- Anthropic adapter tests cover text-run merging with exact block replay,
  server-tool blocks splitting runs in order, fetch results and cited blocks
  replaying in order, truncated cited output replaying as plain text, and
  `pause_turn` continuation with merged usage.
- API projection tests cover citations attached to the preceding message on
  the state and event paths, fetch citation URL resolution within the turn,
  and the hosted search tool step; web transcript tests cover source display.
- OpenAI Responses adapter coverage verifies exact cited-item replay, plain
  replay without the item, and a credentialed hosted-search response with
  real URL citations.
- Ignored direct-provider live tests force each hosted tool independently and
  assert real server-use, server-result, cited-text, and request-version data.

## Acceptance

- A session using `anthropic:messages` may enable search, fetch, or both without
  adding a client-executed web tool.
- Search filters reach Anthropic's native tool request and invalid mixed filter
  lists fail before provider I/O.
- Server-tool state and citation metadata survive durable replay exactly.
- A provider `pause_turn` completes transparently or fails at the bounded
  continuation limit instead of prematurely ending the engine turn.
