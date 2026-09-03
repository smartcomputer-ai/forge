# P153 — Session Labels and Retention

**Status**

- Proposed 2026-09-03. Motivated by the first Harbor evaluation
  ([P149](p149-harbor-end-to-end-agent-evaluation.md)): one job created 89
  sessions in the evaluation universe in 76 minutes, each with only a
  Harbor-derived display name to tell it apart, and nothing to select "all
  sessions of that job" for inspection, cost roll-up, or deletion.
- Registered environments already solve the same problem for environments
  with bounded correlation metadata (P148); sessions get the equivalent, plus
  filtering, bulk operations, and UI.

## Goal

Any client that creates sessions can say where they come from with a few
string labels; anyone can list, inspect, sum, and delete sessions by those
labels, in the API, the CLI, and the Platform UI; and closed sessions that
nobody asked to keep are collected automatically after a retention period.

## Problem

`session/start` accepts a `displayName` and nothing else descriptive.
`session/list` filters only by `parentSessionId` and `rootSessionId` and
orders by last update. `session/delete` takes one closed session. The
Platform sessions page shows a flat list with two toggles (closed,
sub-agents). With hundreds of evaluation, bot, and interactive sessions in
one universe, finding a group means string-matching display names, and
removing a group means one call per session.

## Decision

1. A session carries **labels**: a map of string keys to string values,
   bounded like environment metadata (at most 32 entries, keys up to 64
   bytes, values up to 256, no control characters, no `lightspeed.` prefix).
   Labels are descriptive; they never affect routing, authority, or
   selection.
2. Labels are set at `session/start` and changed with `session/labels/put`
   (replace) and `session/labels/patch` (merge, `null` deletes a key). They
   are part of the session record, not of the event log, so changing them
   does not add transcript entries.
3. `session/list` filters by labels: `labels: {key: value}` with AND
   semantics, and `labelKeys: [key]` for presence. Results carry the labels.
4. Bulk operations take the same filter: `session/delete` gains a
   `filter` form that deletes every closed session matching it, capped per
   call with a continuation, and `session/close` likewise for bulk close.
   A bulk call returns the ids it acted on.
5. Usage rolls up by label: `session/usage/summary` sums tokens and run
   counts over a filter, so a campaign's spend is one call.
6. The Platform sessions page shows labels as chips, filters by them, and
   offers "select all matching" for close and delete.
7. **Closed sessions have a retention period.** A universe sets a default
   (`retention.closedSessionTtlMs`, unset means keep forever) and a label
   filter can override it (`retention.rules: [{filter, ttlMs}]`, first
   match wins; `ttlMs: null` pins a group forever). The lifecycle
   reconciler deletes a closed session once `closedAtMs + ttl` has passed,
   through the ordinary `session/delete` path, so blobs, events, and the
   Temporal history go with it. Nothing is ever collected while open.

## Design

### Records and API

- `sessions` gains a `labels` JSONB column with a GIN index; the in-memory
  store mirrors it. Validation reuses the P148 metadata validator.
- `SessionStartParams.labels`, `SessionSummaryView.labels`,
  `SessionView.labels`; new `SessionLabelsPutParams {sessionId, labels}`
  and `SessionLabelsPatchParams {sessionId, labels}`.
- `SessionListParams` gains `labels` and `labelKeys`; the existing
  `parentSessionId`/`rootSessionId` filters combine with them.
- `SessionDeleteParams` becomes `oneOf {sessionId} | {filter, limit}`;
  `SessionCloseParams` gains the same `filter` form. A filter must name at
  least one label or key; an empty filter is rejected so a bulk call can
  never mean "everything".
- `session/usage/summary {filter}` returns `{sessions, runs, usage:
  LlmUsageView}` summed from run summaries.
- Sub-agent sessions inherit the parent's labels at creation, so a filter
  on a campaign catches its descendants; `origin` still records lineage.

### Retention

- A `universe_settings` row holds `retention` as JSON; `operator/universes/*`
  and a new universe-scoped `settings/retention/put` and `settings/retention/read`
  manage it. Rules use the same label filter as bulk operations.
- The reconciler that already handles ephemeral environment grace (P148)
  gains a session pass: page closed sessions whose `closedAtMs + ttl` has
  elapsed, delete in bounded batches, log one structured event per batch
  with the counts and the rule that matched. Deletion is idempotent and
  survives restarts because it is derived from the rows, not from timers.
- `SessionSummaryView` and `SessionView` expose `expiresAtMs` (derived, null
  when pinned) so the UI can show "collected in 3 days" and a session page
  can offer "keep".
- Evaluation campaigns set a short TTL on their label (`source=harbor`,
  say 14 days) once the report is retained; bots and interactive sessions
  keep the universe default.

### Conventions, not enforcement

- Recommended keys, documented but not validated: `source` (harbor, bot,
  platform, cli), `campaign`, `job`, `task`, `trial`, `bot`, `owner`.
- The Harbor adapter sets `source=harbor`, `job=<job name>`,
  `task=<task>`, `trial=<trial>`, `context=<context id>`, matching the
  metadata it already puts on the registered environment, so a session and
  its environment share keys.
- Bots set `source=bot` and `bot=<bot id>` on the sessions they rotate
  through.

### CLI and UI

- CLI: `lightspeed session list --label source=harbor --label job=…`,
  `session delete --label …`, `session usage --label …`.
- Platform sessions page: label chips on each row, a filter bar that builds
  the same `labels` query, and a selection model over the filtered list with
  close and delete actions and a count confirmation. Labels are editable on
  the session page.

## Acceptance

- Starting a session with labels, listing with a matching filter, patching
  one key, and listing again round-trips; a filter on a missing key returns
  nothing.
- Bulk delete over `job=X` removes only closed sessions with that label,
  returns their ids, and refuses an empty filter.
- Usage summary over a label equals the sum of the per-session run usage.
- A sub-agent started from a labeled session appears under the parent's
  label filter.
- The UI filters a universe with 500 sessions to one job's 89 and deletes
  them in one action.
- A closed session under a 1-hour rule is gone within one reconciler pass
  after the hour; an open session under the same rule is untouched; a
  pinned label survives the universe default.

## Non-Goals

- Labels as authorization or as session grouping for routing.
- Free-text search over transcripts.
- Archival or export before collection; a session that must be kept is
  pinned by a rule or by the universe default.

## Implementation Slices

### Slice 1 — Record, validation, start, put, patch, list

### Slice 2 — Bulk close and delete, usage summary, CLI

### Slice 3 — Platform UI: chips, filter, selection, actions, expiry

### Slice 4 — Retention settings and the reconciler pass

### Slice 5 — Harbor adapter and bots set labels

## Note: `models/list` caching

Unrelated to labels but recorded here at the time of writing: `models/list`
queries every provider live on each call. An evaluation job that starts 12
trials at once issues 12 provider catalog requests within seconds, and the
first hosted trial hit a transient provider 500 that way. A per-universe,
per-provider cache of the discovery result with a TTL of a few seconds (10
to 30 s; a shorter negative TTL for failures, so an outage is retried but
not hammered) collapses the burst to one upstream request without making
the catalog stale in any way a client could notice. Credentials differ per
universe, so the cache key must include the universe.
