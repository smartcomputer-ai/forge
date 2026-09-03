# P153 — Session Metadata

**Status**

- Proposed 2026-09-03. Motivated by the first Harbor evaluation
  ([P149](p149-harbor-end-to-end-agent-evaluation.md)): one job created 89
  sessions in the evaluation universe in 76 minutes, each with only a
  Harbor-derived display name to tell it apart, and nothing to select "all
  sessions of that job" for inspection or cleanup.
- Revised 2026-09-03 after review. The first draft called the map "labels"
  and bundled a patch method, bulk close and delete by filter, a usage
  roll-up, and a retention period. The map is now `metadata`, the concept
  registered environments already carry; patch, bulk operations, and the
  roll-up are dropped (see "Dropped"); retention is its own item,
  [P154](p154-session-retention.md); the `models/list` cache note moved to
  [P155](p155-models-list-cache.md).
- Implemented 2026-09-03, all five slices: `metadata` on the session record
  in every store with migration 013 (`metadata_json` plus GIN indexes on
  sessions and environments), `session/metadata/put`, containment filters on
  `session/list` and `environments/list`, one validator for caller metadata
  (registration bounds plus the reserved prefix; stored records keep the
  bounds only, since registration annotates `lightspeed.envd.version`),
  sub-agent copy at spawn, bots stamping `source=bot` and `bot=<id>`, the
  `lightspeed session` CLI group with looping close and delete, the Platform
  filter bar, chips, selection, and settings-sheet editor, and the Harbor
  adapter passing its correlation map to `session/start`. Unit suites plus
  the Postgres and gateway live tests cover the round trip, containment,
  put, and rejection cases.

## Goal

Any client that creates sessions can say where they come from with a small
string map; anyone can list and inspect sessions by it in the API, the CLI,
and the Platform UI; and environments accept the same filter, so a session
and the environment it ran in are found with one vocabulary.

## Problem

`session/start` accepts a `displayName` and nothing else descriptive.
`session/list` filters only by `parentSessionId` and `rootSessionId` and
orders by last update. The Platform sessions page shows a flat list with
two toggles (closed, sub-agents). With hundreds of evaluation, bot, and
interactive sessions in one universe, finding a group means string-matching
display names.

Registered environments already carry bounded correlation metadata
([P148](p148-key-based-outbound-environment-registration.md)), and the
Harbor adapter stamps `job`, `task`, `trial`, and `context` on every
environment it registers. But `environments/list` cannot filter by it, and
sessions have no equivalent field, so one job cannot be followed from its
sessions to its environments.

## Decision

1. A session carries **`metadata`**: a map of string keys to string values
   with the bounds registered-environment metadata already enforces (at
   most 32 entries, keys 1..=64 bytes, values 1..=256 bytes, no control
   characters, no `lightspeed.` prefix). It is descriptive only and never
   affects routing, authority, or selection. It is the same concept, the
   same name, and the same validator as environment metadata: one word for
   one thing.
2. Metadata is set at `session/start` (and `session/managed/start`) and
   replaced with `session/metadata/put`, which takes the complete map.
   There is no patch: `put` replaces a whole document everywhere else in
   the API (`session/config/put`, `environments/idle-policy/put`), the map
   has at most 32 entries, and every writer named in this item sets it once
   at creation. A client that wants to change one key reads, edits, and
   puts; the last writer wins. Metadata is part of the session record, not
   the event log, and, like `session/rename`, does not touch `updatedAtMs`.
3. `session/list` filters by `metadata: {key: value}` with AND semantics:
   a session matches when it carries every listed pair. The filter combines
   with the existing `parentSessionId` and `rootSessionId` filters, and
   results carry the map. There is no presence-only filter ("has key"): it
   has no named user and would need the wider index (see Design).
4. `environments/list` gains the same `metadata` filter with the same
   semantics.
5. There are no bulk operations. The filtered list is the primitive; the
   CLI and the UI loop over the ids they get back, which they need anyway
   to show a count before acting. Cleaning up after a campaign is
   [P154](p154-session-retention.md)'s job.
6. Sub-agent sessions copy the parent's metadata at spawn, so a filter on a
   campaign catches its descendants; `origin` still records lineage. The
   copy happens once: a later put on the parent does not propagate.
   `rootSessionId` remains the way to reach a tree that has no metadata.

## Design

### Validation

- The strict validator already exists for the registration handshake
  (`validate_registration_metadata` in `environment-protocol`, with the
  bounds as public constants). It becomes the one validator for metadata
  maps: `session/start`, `session/managed/start`, `session/metadata/put`,
  and `environments/create` all call it, and the environments crate's own
  non-empty check goes away. Violations are invalid requests carrying the
  validator's message. The DTO doc comments state the bounds so the
  generated contract carries them.

### Records and stores

- `CreateSession.metadata` and `SessionRecord.metadata`
  (`BTreeMap<String, String>`, empty by default), and a
  `set_session_metadata` store method mirroring `set_session_display_name`:
  it replaces the map and touches neither the log nor `updatedAtMs`.
- `ListSessions.metadata`: the filter map; empty means no filter.
- PostgreSQL:

  ```sql
  ALTER TABLE sessions
      ADD COLUMN IF NOT EXISTS metadata_json jsonb NOT NULL DEFAULT '{}';
  ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_metadata_object;
  ALTER TABLE sessions ADD CONSTRAINT sessions_metadata_object
      CHECK (jsonb_typeof(metadata_json) = 'object');
  CREATE INDEX IF NOT EXISTS sessions_metadata_idx
      ON sessions USING gin (metadata_json jsonb_path_ops);
  CREATE INDEX IF NOT EXISTS environments_metadata_idx
      ON environments USING gin (metadata_json jsonb_path_ops);
  ```

  The whole AND filter is one containment predicate bound as jsonb,
  `metadata_json @> $n`. The planner combines the GIN bitmap with the
  existing `(universe_id, updated_at_ms DESC, session_id DESC)` btree, and
  keyset paging is unchanged. `jsonb_path_ops` indexes containment only,
  which is all the filter needs; a presence filter would need the default
  opclass, which is why it is not in this item.
- The fs store persists the map on the record and, as today, does not
  list. The engine's in-memory store filters in Rust.
- Bump `REQUIRED_SCHEMA_REVISION` and `LIGHTSPEED_SCHEMA_REVISION`
  together.

### API

- `SessionStartParams.metadata`, `ManagedSessionStartParams.metadata`,
  `SessionSummaryView.metadata`, `SessionView.metadata`: all
  `BTreeMap<String, String>`, omitted when empty.
- `session/metadata/put`: `SessionMetadataPutParams {sessionId, metadata}`
  returning `{session: SessionSummaryView}`, like `session/rename`.
- `SessionListParams.metadata` and `EnvironmentListParams.metadata`.
- Regenerate the contract and the TypeScript consumers.

### Conventions, not enforcement

- Recommended keys, documented but not validated: `source` (`harbor`,
  `bot`, `platform`, `cli`), `campaign`, `job`, `task`, `trial`, `bot`,
  `owner`.
- The Harbor adapter sets `source=harbor`, `job=<job name>`,
  `task=<task>`, `trial=<trial>`, `context=<context id>` on the session,
  the same map it already puts on the registered environment, so both are
  found with one filter.
- Bots set `source=bot` and `bot=<bot id>` on the sessions they create,
  routed ones included.

### CLI and UI

- CLI: `--metadata key=value`, repeatable, on `session start`,
  `session list`, and `environment list`; `session metadata put` replaces
  the map; `session close` and `session delete` accept the same filter and
  loop over the matches.
- Platform sessions page: metadata chips on each row, a filter bar that
  builds the `metadata` query, and a selection model over the filtered list
  with close and delete actions that loop over the selection after a count
  confirmation. The map is editable on the session page.

## Acceptance

- Starting a session with metadata, listing with a matching filter, putting
  a new map, and listing again round-trips; a filter naming a key the
  session lacks returns nothing; a two-key filter requires both pairs.
- Start and put reject more than 32 entries, an empty key or value, a
  control character, and a `lightspeed.` key.
- `environments/list` with the filter the Harbor adapter uses returns the
  environment registered for that trial.
- A sub-agent spawned from a session with metadata appears under the
  parent's filter.
- The UI filters a universe with 500 sessions to one job's 89 and closes
  or deletes them from the selection.
- The committed contract and generated clients are current.

## Non-Goals

- Metadata as authorization, routing, or grouping for selection.
- Free-text search over transcripts.
- Retention and reaping ([P154](p154-session-retention.md)).
- Usage roll-up per filter. Usage lives only in run and turn event
  payloads; summing it over a filter is a query over `session_events` and
  its own item when someone needs it.

## Dropped

Reviewed and rejected; do not re-propose:

- The name `labels`. Environments already call the same bounded map
  `metadata`.
- `session/labels/patch`. `put` replaces, and the map is small.
- A `labelKeys` presence filter.
- Bulk `session/close` and `session/delete` taking `oneOf {sessionId} |
  {filter, limit}`. No method in the API takes a union, and the loop is
  cheap.
- `session/usage/summary`.
- Retention rules keyed on metadata.

## Implementation Slices

### Slice 1 — Validator, record, stores, start, put, list filter

### Slice 2 — `environments/list` filter and index

### Slice 3 — CLI

### Slice 4 — Platform UI: chips, filter bar, selection, editing

### Slice 5 — Harbor adapter and bots set metadata
