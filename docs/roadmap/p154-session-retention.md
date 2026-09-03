# P154 — Session Retention

**Status**

- Proposed 2026-09-03, split out of [P153](p153-session-metadata.md) so
  metadata and lifecycle stay separate items. Motivated by the same Harbor
  evaluation ([P149](p149-harbor-end-to-end-agent-evaluation.md)): every
  session ever closed in a universe stays until someone deletes it by hand.
- Also records a defect found while settling the vocabulary: the bot
  routed-session "TTL" is documented as an idle timer but runs from
  creation (see "Related defect").

## Vocabulary

Three clocks are easy to conflate, and the codebase already uses two of
them:

- **TTL** counts from creation: a cache entry or a DNS record lives this
  long and then expires, whatever happened in between. The one correct TTL
  in this repository is the `models/list` cache
  ([P155](p155-models-list-cache.md)).
- **Idle policy** counts from the last activity and turns *open* into
  *closed* (or paused, suspended, stopped). Environments have it as
  `idlePolicy.closeAfterMs` and siblings. Bots have it for their routed
  sessions as `routedSessionTtlMs` and the trigger's `sessionTtlMs`, which
  is the wrong name for it.
- **Retention** counts from close and turns *closed* into *gone*: how long
  a finished record is kept before deletion. Temporal's namespace retention
  has exactly these semantics, and sessions are workflow-backed.

This item is retention. It never closes an open session, and it does not
introduce a universe-wide idle policy for sessions (see "Non-Goals").

## Goal

A closed session that nobody asked to keep is deleted automatically after a
universe-wide retention period, through the same path as `session/delete`,
so its rows, events, and checkpoint go with it and its owned environments
are closed; a session anyone wants to keep is kept with one flag; and the
UI shows when a closed session will be collected.

## Problem

Nothing deletes sessions. `session/delete` takes one closed session, so
every evaluation, bot, and interactive session accumulates until an
operator loops over them. Temporal forgets the workflow history after its
namespace retention, but the session row, its event log, and its checkpoint
stay forever.

## Decision

1. **A universe has a default retention for closed sessions:**
   `closedSessionRetentionMs`, nullable. Null, the default, keeps closed
   sessions forever; when set it must be greater than zero. There is no
   zero-as-forever sentinel.
2. **A session can be kept.** A `keep` flag, false by default, set with
   `session/retention/put {sessionId, keep}`. A kept session is never
   collected. Clearing the flag puts it back under the universe default.
3. **Close time is recorded.** `closedAtMs` is stamped in the same write
   that records `closedAtSeq`, from the closing event's observation time.
   Forks inherit it the way they inherit the closing sequence. Existing
   closed rows are backfilled from `updatedAtMs`.
4. **Expiry is derived, not stored.** `expiresAtMs = closedAtMs +
   closedSessionRetentionMs` when the session is closed, not kept, and the
   universe has a default; otherwise absent. Changing the universe default
   therefore applies to sessions that are already closed at the next pass.
   That is intended: "delete closed sessions after 30 days" means all of
   them.
5. **A reaper pass collects them.** It runs on the worker role beside the
   promise reaper, which already pages every session of every universe on
   a fixed interval. Per universe it reads the default, pages closed,
   unkept sessions whose expiry has passed, oldest first, in bounded
   batches, and deletes each through the same function `session/delete`
   uses, never the store directly, so owned environments are closed exactly
   as for a manual delete. It is idempotent and restart-safe because it
   derives everything from rows. It logs one structured event per batch
   with the universe, the counts, and the ids it skipped.
6. **Some closed sessions cannot be deleted yet.** The store refuses to
   delete a session a fork still reads from. The reaper skips those, logs
   them, and moves on; they are collected once the fork is gone.
7. **The bot timer is renamed to what it measures.** `routedSessionTtlMs`
   becomes `routedSessionCloseAfterMs` and the trigger's `sessionTtlMs`
   becomes `sessionCloseAfterMs`, matching the environment idle policy, and
   the clock is refreshed on every routed event (see "Related defect").
   Greenfield rename with regenerated clients, no compatibility.

## Design

### Schema

```sql
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS closed_at_ms bigint;
UPDATE sessions SET closed_at_ms = updated_at_ms
    WHERE closed_at_seq IS NOT NULL AND closed_at_ms IS NULL;
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_closed_at_pair;
ALTER TABLE sessions ADD CONSTRAINT sessions_closed_at_pair
    CHECK ((closed_at_ms IS NULL) = (closed_at_seq IS NULL));

ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS keep boolean NOT NULL DEFAULT false;

-- The reaper's scan: closed, unkept, oldest close first.
CREATE INDEX IF NOT EXISTS sessions_retention_idx
    ON sessions (universe_id, closed_at_ms)
    WHERE lifecycle_status = 'closed' AND NOT keep;

ALTER TABLE universes
    ADD COLUMN IF NOT EXISTS closed_session_retention_ms bigint;
ALTER TABLE universes DROP CONSTRAINT IF EXISTS universes_retention_positive;
ALTER TABLE universes ADD CONSTRAINT universes_retention_positive
    CHECK (closed_session_retention_ms IS NULL
           OR closed_session_retention_ms > 0);
```

The reaper's per-universe query is the index, verbatim:

```sql
SELECT session_id
FROM sessions
WHERE universe_id = $1
  AND lifecycle_status = 'closed'
  AND NOT keep
  AND closed_at_ms <= $2   -- now minus the universe default
ORDER BY closed_at_ms
LIMIT $3
```

Bump `REQUIRED_SCHEMA_REVISION` and `LIGHTSPEED_SCHEMA_REVISION` together.

### Records and stores

- `SessionRecord.closed_at_ms` and `SessionRecord.keep`; the close path
  stamps `closed_at_ms` wherever it stamps `closed_at_seq`, including the
  fork derivation.
- `set_session_keep`, mirroring `set_session_display_name`: record only,
  no log, no `updatedAtMs`.
- `list_sessions_due_for_deletion(cutoff_ms, limit)`: the query above.
- The universe row gains the default; `list_universes` returns it and a
  put writes it.

### API

- `session/retention/put`: `SessionRetentionPutParams {sessionId, keep}`
  returning `{session: SessionSummaryView}`, like `session/rename`.
- `SessionSummaryView` and `SessionView` gain `closedAtMs` (absent while
  open), `keep`, and `expiresAtMs` (absent when open, kept, or the universe
  has no default).
- `operator/universes/put`: a full-document put in the style of
  `bots/put`, `{universe: {universeId, slug, closedSessionRetentionMs}}`,
  creating or replacing. `OperatorUniverseView` gains
  `closedSessionRetentionMs`. Universe rows are operator-owned today and
  nothing else configures a universe; if universes ever get self-service
  settings the field moves with them.
- Bot rename per Decision 7 in `BotInput`, `BotView`, and the trigger
  documents, plus the `bot_trigger_put` tool argument.

### Reaper

- Lives beside the promise reaper in the worker role and shares its
  interval convention (documented in `docs/variables.md`). Per pass and
  universe it collects at most one batch (256, the promise reaper's page);
  the rest waits for the next pass, so a universe with ten thousand expired
  sessions drains steadily instead of in one transaction storm.
- The delete step, store delete followed by closing owned environments,
  moves out of the gateway service into a function both the service and the
  reaper call. If closing owned environments turns out to need adapters
  only the gateway holds, the pass moves to the role that has them rather
  than dispatching across roles.
- Skips are `SessionNotClosed` (raced with a reopen, impossible today but
  cheap to tolerate) and the fork-child refusal; both are logged with the
  session id and never fail the batch.

### CLI and UI

- CLI: `session keep <id>` and `session keep --off <id>`;
  `operator universes put` for the default.
- Session page: "Collected in 3 days" from `expiresAtMs` and a Keep toggle.
  Sessions list: the same hint on closed rows. Admin universes page: the
  retention field.

## Related defect: the bot routed-session clock

`routedSessionTtlMs` is documented as "close routed sessions idle longer
than this"; the controller computes expiry as `lastActiveAtMs + ttl` and
skips busy sessions. But `lastActiveAtMs` is stamped only when the routed
session is created and refreshed only when a close attempt fails. When a
later event lands on a known routed session, `ensure_routed_session` in
`crates/temporal-workflow/src/workflows/bots/controller/lanes.rs` updates
the session's TTL and leaves the clock alone. So a per-key session that
keeps receiving events still closes at creation plus TTL the first time it
is idle, and the extra-session cap, which evicts by the same stamp, removes
the oldest-created session rather than the least recently used.

Fix: touch the clock in the known-session branch, and preferably also when
the session stops being busy, so "idle" means idle since the last run
rather than since the last event. The rename in Decision 7 rides along so
the name says what the clock measures. This ships independently of the
reaper and can go first.

## Acceptance

- Under a one-hour universe default, a session closed more than an hour ago
  is gone after one pass, an open session older than that is untouched, a
  kept session survives, a closed session with a live fork is skipped and
  logged, and a second pass is a no-op.
- Deleting through the reaper closes the session's owned environments
  exactly as `session/delete` does.
- Under a null default nothing is deleted; setting a default afterwards
  collects already-closed sessions at the next pass.
- `expiresAtMs` equals `closedAtMs` plus the default, and is absent when
  the session is open, kept, or the universe has no default.
- A routed per-key bot session that receives a second event halfway through
  its window survives past the original deadline; cap eviction picks the
  least recently active session.

## Non-Goals

- Closing open sessions on any clock. A universe-wide session idle policy
  (a universe default plus a per-session override measured from
  `updatedAtMs`, closing after N idle) would be a separate item and could
  subsume the bot timer.
- Retention rules keyed on metadata.
- Archival or export before deletion.
- Deleting CAS blobs; they are content-addressed and shared.

## Dropped

Reviewed and rejected; do not re-propose:

- "TTL" as the name for any of this.
- A per-session retention period. The universe default and `keep` cover
  every acceptance case; a universe that mixes lifetimes is two universes.
- Rules `[{filter, ttlMs}]` keyed on metadata with first-match-wins.
- A `universe_settings` table and `settings/retention/put|read`; the
  universe row and `operator/universes/put` carry the one field.
- Zero as "forever".
- Running the pass in the environment-registration reconciler; it is
  environment-specific and lives on the gateway.

## Implementation Slices

### Slice 1 — `closedAtMs`, `keep`, views, `session/retention/put`, backfill

### Slice 2 — Universe default: column, `operator/universes/put`, view

### Slice 3 — Reaper pass and its live test

### Slice 4 — Platform UI and CLI

### Slice 5 — Bot clock fix and rename (independent; can go first)
