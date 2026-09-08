# P154 — Session Retention

**Status**

- Implemented 2026-09-03. Split out of [P153](p153-session-metadata.md) so
  metadata and lifecycle stay separate items. Motivated by the same Harbor
  evaluation ([P149](p149-harbor-end-to-end-agent-evaluation.md)): every
  session ever closed stays until someone deletes it by hand.
- Also records a defect found while settling the vocabulary: the bot
  routed-session "TTL" is documented as an idle timer but runs from creation
  (see "Related defect").
- Extended 2026-09-04 with profile creation defaults. A profile carries an
  optional `retention: ProfileSessionRetention`; omission at session start
  inherits it, an explicit duration overrides it, and explicit null keeps the
  new root tree until manual deletion.

## Vocabulary

Three clocks are easy to conflate, and the codebase already uses two of them:

- **TTL** counts from creation: a cache entry or a DNS record lives this long
  and then expires, whatever happened in between. The one correct TTL in this
  repository is the `models/list` cache
  ([P155](p155-models-list-cache.md)).
- **Idle policy** counts from the last activity and turns *open* into *closed*
  (or paused, suspended, stopped). Environments have it as
  `idlePolicy.closeAfterMs` and siblings. Bots historically called the same
  concept `routedSessionTtlMs` and `sessionTtlMs`; this item gives those fields
  accurate names.
- **Retention** counts from close and turns *closed* into *gone*: how long a
  finished record is kept before deletion. Temporal's namespace retention has
  exactly these semantics, and sessions are workflow-backed.

This item is retention. It never closes an open session, and it does not
introduce a universe-wide policy. Automatic deletion is an explicit choice on
a session tree; an unset policy means the tree is kept until an operator
deletes it manually.

## Goal

A root session can opt its whole owned session tree into automatic deletion a
fixed time after the root closes. History forks and delegated sub-agent
children belong to that tree; config-only clones do not. Descendants do not
carry competing policies or clocks. A root without a policy, and therefore its
tree, is kept by default.

Automatic deletion and explicitly cascading manual deletion remove a selected
session subtree through one shared lifecycle path, so rows, events, and
checkpoints go together and every deleted session's owned environments are
closed. Ordinary manual deletion remains a conservative one-session operation.
Session settings identify the retention root and show the tree's deletion
policy without crowding the primary list or detail header.

## Problem

Nothing deletes sessions automatically. `session/delete` currently removes
one closed session. It refuses to delete a history source while a fork still
reads inherited events from it, but it neither deletes nor is blocked by
delegated sub-agent children. Evaluations, bots, and other callers that create
disposable session trees must discover and delete the leaves in the right
order themselves.

Copying a duration onto every descendant does not solve that cleanly. Each
session would acquire a different close-relative deadline, users could change
one copy independently, and a retained child could indefinitely prevent its
fork source from being deleted. The lifecycle unit is the owned tree, so the
tree needs one policy and one deadline.

A universe-wide default is also too broad. Different trees in the same
universe can have different owners and purposes, and permanent retention is
the safer default. The caller creating a disposable root already knows that
intent and should record it on that root.

## Decision

1. **Retention has one owner.** Every session records an immutable
   `retentionRootSessionId`. A fresh session owns itself. A history fork
   inherits its source's root. A delegated session with `origin` inherits its
   parent's root. A config-only clone starts a new retention tree and owns
   itself because its event history is independent.
2. **Automatic deletion is configured only on the root.** A retention root
   has nullable `deleteAfterCloseMs`. Null, the default, means no automatic
   deletion. A positive value means delete the tree that many milliseconds
   after the root closes. Descendants do not store an independent policy.
   There is no separate `keep` flag, zero-as-forever sentinel, or universe
   default.
3. **The policy is available at creation and remains editable.**
   `ProfileDocument.retention` may provide
   `ProfileSessionRetention { deleteAfterCloseMs }` as a creation default;
   applying a profile to an existing session never changes retention.
   `session/start` and `session/managed/start` accept
   `deleteAfterCloseMs`: omission inherits the profile, explicit null keeps
   the tree, and an explicit duration overrides the profile.
   `session/retention/put {sessionId,
   deleteAfterCloseMs}` replaces it later. The mutation accepts only a
   retention root; targeting a descendant is rejected with its root id rather
   than silently changing an ancestor. Null clears the policy and keeps the
   tree.
4. **Close time is recorded per session.** `closedAtMs` is stamped in the same
   write that records `closedAtSeq`, from the closing event's observation
   time. Forks inherit it when their inherited prefix includes the close
   event. Existing closed rows are backfilled from `updatedAtMs`. Reopening
   clears it and the next close stamps a new close time.
5. **The tree deadline comes only from the root.** For a closed retention root
   with a policy, `deleteAtMs = root.closedAtMs + deleteAfterCloseMs`;
   otherwise it is absent. Descendant close times do not create deadlines.
   Setting or changing the policy on an already-closed root uses the original
   close time. A deadline already in the past makes the tree eligible for the
   next reaper pass; clearing or extending the policy before deletion wins.
6. **Manual deletion is conservative unless cascade is explicit.**
   `session/delete {sessionId}` deletes only that closed session and therefore
   requires it to be a leaf of the retention tree.
   `session/delete {sessionId, cascade: true}` expands the target through
   history-fork edges and `origin.parentSessionId` edges, requires every member
   of that subtree to be closed, and deletes descendants before the target.
   Config-only clones are excluded in both modes and survive with their content
   source link cleared. The default call rejects any target with a surviving
   history fork or `origin` child. It does not orphan children, even when they
   are storage-independent, because that would break the retention-tree
   ownership invariant.
7. **Automatic collection operates on whole retention trees.** A worker-role
   reaper visits each universe, pages due root sessions in deadline order, and
   invokes the same path as `session/delete` with cascade enabled. Timed
   deletion therefore always applies to the whole tree. If any tree member is
   open, the pass skips the root instead of force-closing active work.
   Eligibility and tree membership are rechecked under the deletion transaction
   so policy changes and new children cannot race into partial deletion.
8. **The bot timer is renamed to what it measures.** `routedSessionTtlMs`
   becomes `routedSessionCloseAfterMs` and the trigger's `sessionTtlMs`
   becomes `sessionCloseAfterMs`, matching the environment idle policy. Its
   clock is refreshed on every routed event (see "Related defect"). Greenfield
   rename with regenerated clients, no compatibility.

## Retention Tree

Retention ownership follows the relationship that makes a session part of the
same body of work:

| Session creation | Retention root | Deleted with parent/source |
| --- | --- | --- |
| Fresh session | itself | not applicable |
| History fork (`sourceSeq` set) | source's retention root | yes |
| Delegated child (`origin` set) | parent's retention root | yes |
| Config-only clone (`sourceSeq` absent) | itself | no |

The stored root id makes the effective policy unambiguous even when history
forks and delegation are nested. Creation must resolve the root from the
parent/source row in the same transaction. A session may not name conflicting
retention parents.

The existing `origin` document remains immutable provenance, but its parent
edge additionally defines retention containment. Clone/fork
`sourceSessionId` remains content lineage: only `sourceSeq` being present
makes that edge retention containment. A clone is the escape hatch for work
that must survive deletion of the source; a history fork cannot outlive the
events it reads by reference.

## Design

### Schema

The core schema baseline defines the close projection, immutable tree root,
and root-only policy/deadline directly on the session row. Relevant column and
constraint definitions:

```sql
closed_at_ms bigint,
retention_root_session_id text NOT NULL,
delete_after_close_ms bigint,
delete_at_ms bigint GENERATED ALWAYS AS
    (closed_at_ms + delete_after_close_ms) STORED,

CONSTRAINT sessions_closed_at_pair
    CHECK ((closed_at_ms IS NULL) = (closed_at_seq IS NULL)),
CONSTRAINT sessions_delete_after_close_positive
    CHECK (delete_after_close_ms IS NULL OR delete_after_close_ms > 0),
CONSTRAINT sessions_retention_policy_on_root
    CHECK (retention_root_session_id = session_id OR delete_after_close_ms IS NULL)
```

The retention indexes are:

```sql
CREATE INDEX IF NOT EXISTS sessions_retention_root_idx
    ON sessions (universe_id, retention_root_session_id, session_id);
CREATE INDEX IF NOT EXISTS sessions_retention_due_idx
    ON sessions (universe_id, delete_at_ms, session_id)
    WHERE lifecycle_status = 'closed' AND delete_at_ms IS NOT NULL;
```

Fresh sessions and clones resolve the retention root to themselves; history
forks and delegated children inherit their owner's root. The original upgrade
backfill was removed when the greenfield schema was consolidated into the core
baseline; existing development databases are recreated for that baseline.

PostgreSQL generates `delete_at_ms` from the close time and retention duration.
The stored result gives the reaper a direct ordered index without requiring
writers to synchronize another field. Null inputs clear the deadline for open
sessions, roots without a policy, and descendants. PostgreSQL rejects bigint
overflow atomically. Descendant API views obtain the effective policy and
deadline by joining their stored root id.

The reaper query reads only roots with a due deadline:

```sql
SELECT session_id
FROM sessions
WHERE universe_id = $1
  AND retention_root_session_id = session_id
  AND lifecycle_status = 'closed'
  AND delete_at_ms IS NOT NULL
  AND delete_at_ms <= $2
ORDER BY delete_at_ms, session_id
LIMIT $3
```

Bump `REQUIRED_SCHEMA_REVISION` and `LIGHTSPEED_SCHEMA_REVISION` together.

### Records and stores

- `SessionRecord.closed_at_ms`, `retention_root_session_id`, and the root-only
  `delete_after_close_ms` and `delete_at_ms`.
- Fresh and clone creation set the new session id as the retention root.
  History-fork and delegated-child creation lock their source/parent and copy
  its root id. Creation also locks the retention root so it cannot commit a
  new tree member concurrently with deletion.
- The close and reopen paths maintain `closed_at_ms`; PostgreSQL generates or
  clears `delete_at_ms` in the same write. The in-memory store maintains the
  equivalent derived value in its record projection.
- `set_session_retention` locks the root, rejects a descendant target, and
  replaces the nullable duration without touching the event log or `updatedAtMs`.
  PostgreSQL generates the deadline and returns it with the updated record.
- `list_retention_roots_due_for_deletion(now_ms, limit)` uses the
  universe-scoped query above.
- `delete_closed_sessions(session_id, cascade, due_at_or_before)` returns the
  target record and all deleted session ids. Cascade mode locks the target's
  retention root, expands both containment edge kinds recursively, locks every
  member, checks that all are closed, optionally rechecks the root deadline,
  and deletes the subtree in one transaction. Non-cascade mode locks and
  deletes only a leaf target after verifying that it has no history-fork or
  `origin` descendants. The in-memory store implements the same ordering and
  checks.
- Config-only clone edges are not traversed. Their `source_session_id` is
  cleared by the existing `ON DELETE SET NULL` behavior when the source row is
  deleted.

### API

- `ProfileDocument.retention?: ProfileSessionRetention` where
  `ProfileSessionRetention = { deleteAfterCloseMs }`. The nested document is
  intentionally extensible and is used only when creating a root session.
- `SessionStartParams.deleteAfterCloseMs` and
  `ManagedSessionStartParams.deleteAfterCloseMs`: optional nullable
  root-creation overrides; absent inherits the profile, null disables its
  default, and a positive duration replaces it.
- `session/retention/put`: `SessionRetentionPutParams {sessionId,
  deleteAfterCloseMs}` returning `{session: SessionSummaryView}`, like
  `session/rename`. The field is nullable and required in this full-document
  put: a number enables or changes deletion and null disables it. A descendant
  target returns a typed error containing `retentionRootSessionId`.
- `SessionSummaryView` and `SessionView` gain `closedAtMs` and a
  `retention: SessionRetentionView` projection:

  ```text
  SessionRetentionView = {
    rootSessionId,
    deleteAfterCloseMs?,
    deleteAtMs?
  }
  ```

  The projection is effective for the viewed session: descendants name the
  same root and see the same optional policy and deadline. `deleteAtMs` is
  absent while the root is open or automatic deletion is disabled.
- `session/delete` gains optional `SessionDeleteParams.cascade`, false by
  default. False requests one-session deletion; true requests deletion of the
  target's full fork/origin subtree. The response retains the deleted target
  summary and adds `deletedSessionCount`; it does not return an unbounded id
  list.
- There is no universe retention field, operator API, descendant override, or
  implicit cascade.

### Shared deletion path

- Store deletion and closing owned environments move out of the gateway
  method into a lifecycle function shared by manual deletion and the reaper.
- The function closes owned `closeWithSession` environments for every deleted
  session, not just the requested target. This is best effort after the store
  transaction; the environment reconciler also treats a missing origin
  session as closed and converges any missed work.
- A non-cascading manual call has no deadline precondition and requires only
  its target to be closed, but also requires it to have no history-fork or
  `origin` descendants. A cascading manual call requires every selected
  descendant to be closed. Either mode fails atomically without removing rows.
- A reaper call always enables cascade and supplies the scanned root and
  deadline. It fails atomically if the policy was cleared or extended, the
  root reopened, a tree member is open, or membership changed before the
  transaction acquired its locks.
- Deletion logs the requested target, retention root, number of sessions
  deleted, and whether the caller was manual or retention. It does not emit an
  unbounded list at normal log levels.

### Reaper

- Lives beside the promise reaper in the worker role and shares its interval
  convention documented in `docs/variables.md`.
- Each pass collects at most one batch of 256 roots per universe, the promise
  reaper's page size. The rest waits for the next pass, so a large backlog
  drains steadily instead of producing one transaction storm.
- An open descendant, concurrent policy/lifecycle change, or membership race
  skips that root and never fails the batch. Structured pass statistics count
  scanned roots, deleted roots, deleted sessions, open-tree skips, conflicts,
  and errors.
- A skipped due root remains indexed and is retried on the next pass. Subagent
  executions normally close their child sessions on every terminal path, so a
  healthy tree converges without the reaper force-closing anything.

### CLI and UI

- CLI: set or clear `deleteAfterCloseMs` on a retention root. Accept a human
  duration for convenience, but send milliseconds on the wire. A descendant
  error prints the root id to edit instead.
- Session page: an automatic-deletion control on roots, with "Keep until
  manually deleted" as the default. Descendants show "Retained with <root>"
  and link to the root rather than presenting an editable control.
- Session settings show the effective policy and retention root. A root can
  set or clear its close-relative duration; a descendant links to the root
  rather than presenting an editable control. The primary sessions list and
  detail header stay focused on identity, lifecycle state, and actions.
- Profile, customized session-create, and bot-profile editors show Automatic
  deletion as a collapsed card immediately after Session metadata. Clearing
  the days field removes the profile retention section.
- Manual deletion defaults to the selected session only. The UI offers an
  explicit "also delete forks and delegated children" option with a bounded
  descendant count. For any non-leaf target, it explains that cascade is
  required; children cannot be orphaned. Config-only clones are never included.
- There is no admin-universe retention setting.

## Related defect: the bot routed-session clock

The former `routedSessionTtlMs` field was documented as "close routed sessions
idle longer than this"; the controller computed expiry as
`lastActiveAtMs + duration` and skipped busy
sessions. But `lastActiveAtMs` is stamped only when the routed session is
created and refreshed only when a close attempt fails. When a later event
lands on a known routed session, `ensure_routed_session` in
`crates/temporal-workflow/src/workflows/bots/controller/lanes.rs` updates the
session's close policy and leaves the clock alone. So a per-key session that
keeps receiving events still closes at creation plus the duration the first time it is idle,
and the extra-session cap, which evicts by the same stamp, removes the
oldest-created session rather than the least recently used.

Fix: touch the clock in the known-session branch, and preferably also when the
session stops being busy, so "idle" means idle since the last run rather than
since the last event. The rename in Decision 8 rides along so the name says
what the clock measures. This ships independently of the reaper and can go
first.

## Acceptance

- A fresh session and a config-only clone each identify themselves as their
  retention root. A nested history fork and delegated sub-agent child identify
  the original owning root.
- A root created from a profile inherits its retention default when the start
  override is absent; a duration overrides it and explicit null clears it.
  Delegated children ignore profile defaults and retain the parent root policy.
- A root with `deleteAfterCloseMs = 3_600_000` is due after its close time is
  more than an hour old. Its descendants expose the same effective
  `deleteAtMs`; their own close times do not move it.
- A tree whose root has no deletion policy survives every reaper pass. Existing
  trees have no policy after migration.
- An open root is never deleted. A closed due root with any open descendant is
  skipped without deleting any member; after the descendant closes, the next
  pass deletes the whole tree.
- Setting a policy on an already-closed root derives the deadline from its
  original close time. Clearing or extending it before the conditional delete
  prevents collection.
- Setting retention through a descendant is rejected with the actual root id;
  there is never more than one applicable policy.
- Automatic deletion of a root behaves as cascade enabled: it deletes nested
  history forks and `origin` children leaf-first and closes every deleted
  session's owned environments.
- Manual deletion without cascade removes only the target. It rejects a
  target with any surviving history-fork or `origin` descendant, whether or
  not the target is the retention root.
- Manual deletion with cascade removes the target's fork and `origin` subtree,
  but not its ancestors, siblings, or config-only clones.
- Config-only clones survive deletion of their source and retain their own
  independent retention policy. Their source link is cleared.
- A concurrent child creation, reopen, or retention change cannot cause a
  partial tree deletion. A repeated successful reaper pass is a no-op.
- A routed per-key bot session that receives a second event halfway through
  its window survives past the original deadline; cap eviction picks the
  least recently active session.

## Non-Goals

- A universe-wide session-retention default.
- Independent retention policies on history forks or delegated children.
- Closing open sessions on a retention clock. A universe-wide session idle
  policy measured from `updatedAtMs` would be a separate item and could
  subsume the bot timer.
- Treating config-only clones as owned descendants.
- Retention rules keyed on metadata.
- Archival or export before deletion.
- Deleting CAS blobs; they are content-addressed and shared.

## Dropped

Reviewed and rejected; do not re-propose:

- "TTL" as the name for any of this.
- A universe-wide `closedSessionRetentionMs` plus a per-session `keep` flag.
- A separate `keep` flag. Null root `deleteAfterCloseMs` already means keep
  until manual deletion.
- Copying `deleteAfterCloseMs` onto forks and delegated children. That creates
  competing close-relative deadlines and makes source cleanup depend on which
  copy wins.
- Allowing a descendant retention mutation to silently modify its root.
- Implicit cascade for manual deletion. Destructive tree expansion must be
  requested explicitly; timed retention is the explicit tree-level opt-in.
- An `orphanChildren` or equivalent deletion flag. Retention containment is an
  ownership invariant; independent work should start as a config-only clone
  with its own retention root. Detaching an existing descendant would require
  a separate explicit materialization operation, not a delete option.
- Cascading deletion into config-only clones. Their log is independent and the
  clone is the explicit way to detach work from the source tree.
- Rules `[{filter, ttlMs}]` keyed on metadata with first-match-wins.
- A `universe_settings` table and `settings/retention/put|read`.
- Zero as "forever".
- Running the pass in the environment-registration reconciler; it is
  environment-specific and lives on the gateway.

## Implementation Slices

### Slice 1 — Retention-root projection, backfill, and tree deletion

### Slice 2 — Close time, root policy, effective views, and retention API

### Slice 3 — Reaper pass and its live test

### Slice 4 — Platform UI and CLI

### Slice 5 — Bot clock fix and rename (independent; can go first)
