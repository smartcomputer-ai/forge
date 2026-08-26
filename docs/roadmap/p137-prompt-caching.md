# P137 — Prompt Caching: Make It Exist, Keep It Warm, Prove It

**Status**

- Proposed 2026-08-26, surfaced by the P135 review. Deliberately short
  until it is closer; the facts below were verified in code the same day.
- Depends on [P136](p136-context-catalogs.md) for catalog refreshes to
  stop breaking the prefix; independent of it for everything else.
- Core tier: `llm-runtime` adapters, `llm-clients` request types, `api`
  views for usage.

## Goal

A long-lived session pays for its history once. Concretely: on the second
and later turns of a session — and on the first turn of a later run in the
same session — the provider reports cached input tokens for the stable
prefix, and the ordinary things that happen to a session (a new user
message, a tool result, a bot event delivery, a catalog refresh under P136)
do not drop the hit rate to zero. The dominant cost of a months-old bot
session is re-reading its history; this is the lever.

## Facts today (verified 2026-08-26)

- **Anthropic: no caching at all in production.** Anthropic caches only at
  explicit `cache_control` breakpoints. The adapter sets `cache_control:
  None` on every block, materializes the system prompt as
  `SystemContent::Text` (no block to mark), and never sets the field on a
  tool definition. Per-entry provider options can carry `cache_control`
  (covered by a unit test) but nothing in the product sets them. The client
  already parses `cache_creation_input_tokens` / `cache_read_input_tokens`
  into the turn's `LlmUsage` (`cached_input_tokens`,
  `cache_write_input_tokens`).
- **OpenAI: automatic prefix caching, full resend.** Responses and
  Completions cache prefixes automatically (≥ 1024 tokens). The adapter
  resends the whole input every turn — `provider_response_id` is "currently
  always `None`" by design, which is correct: server-side chaining would
  fight context edits and replay. `cached_tokens` is parsed into
  `LlmUsage`. No `prompt_cache_key` is sent, so cache routing across
  OpenAI's fleet is best-effort.
- **Usage is invisible.** `LlmUsage` lives on the engine's turn facts and
  is not exposed by `api`; neither the UI nor a test can read a hit rate
  without the session log.
- **Prefix stability.** Tail appends (user input, tool results, bots'
  `append`/steer deliveries, P135 directory deltas) keep the prefix.
  Catalog upserts (P136), instruction rewrites, and compaction rewrite it;
  the first is fixable, the other two are deliberate.

## Proposed

1. **Anthropic breakpoints, placed by the adapter.** In
   `anthropic_messages.rs`, materialize the system prompt as `Blocks` with
   `cache_control` on its last block, set `cache_control` on the last tool
   definition, and put a moving breakpoint on the last content block of
   the last message — the standard three-breakpoint layout (limit four).
   Placement is a materialization detail: nothing in the session log or the
   planned request changes. TTL: default `5m`; `1h` behind a runtime config
   knob for sessions that wake rarely (bots), where the higher write price
   pays for itself.
2. **OpenAI `prompt_cache_key`.** Send the session id on Responses and
   Completions so the cache is routed consistently; keep full resend.
3. **Expose usage.** Add an `LlmUsage` view (input, output, cached,
   cache-write) to run and turn views and `session/events/read`; regenerate
   the contract; show the cached share per run in the web session view and
   per delivery in the bot detail.
4. **A cheap regression detector.** Per session, derive the hit ratio of
   the last turn; log at warn when a turn with a large input reports zero
   cached tokens right after a turn that reported hits — the signature of a
   broken prefix (a P136-class regression, an accidental instructions
   rewrite).

## Proving it works

The point of the doc: caching must be tested, not assumed.

- **Deterministic, no provider** (`llm-runtime` unit tests): materialize
  request N and N+1 and assert N's message list is a byte-level prefix of
  N+1's for every tail-append case (user message, tool result, keyed
  append, P136 supersede) and *not* for the expected breaks (instructions
  rewrite, compaction). Assert the Anthropic breakpoints land where the
  layout says.
- **Live, per provider** (`crates/llm-runtime/tests/*_caching_live.rs`,
  ignored, run with the other live suites):
  - two turns in one run → turn 2 reports cached input ≥ 80 % of turn 1's
    input (Anthropic: turn 1 reports a cache write);
  - run 1 then run 2 in the same session → run 2's first turn still hits;
  - a tool-call round trip in between → the hit holds;
  - with P136: a catalog upsert between turns → the hit holds;
  - a compaction → the next turn re-warms (a write, then hits again).

## Non-goals

- Server-side conversation chaining (`previous_response_id`).
- Provider-managed compaction changes.
- Cost accounting or billing; this doc only makes hit rates real and
  visible.
