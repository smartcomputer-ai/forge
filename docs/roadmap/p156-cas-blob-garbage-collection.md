# P156 — CAS Blob Garbage Collection

**Status**

- Implemented 2026-09-03, all four slices, in schema revision 15. See
  "Implementation Notes" at the end for what differed from the proposal.
- Proposed 2026-09-03 after reviewing what session deletion
  ([P154](p154-session-retention.md)) leaves behind. Retention now removes
  rows, events, and checkpoints, but every blob a deleted session wrote stays
  in `cas_blobs` and the object store forever. The Harbor evaluation
  ([P149](p149-harbor-end-to-end-agent-evaluation.md)) creates hundreds of
  disposable sessions per run, so the store grows by the full size of every
  context the model ever saw.
- Also records two write-only leaks that need no collector: raw provider
  request/response dumps and superseded checkpoint state. Closing the first
  one alone removes more than half of the bytes written today.

## Vocabulary

- **Blob**: one immutable, content-addressed payload in `cas_blobs`, inline in
  Postgres under the inline threshold and in the object store above it.
- **Holder**: a durable row that references a blob and must be able to read
  it later. Holders are the roots of reachability.
- **Edge**: a recorded parent-to-child relationship between two blobs whose
  parent bytes embed the child's ref. Edges make nested formats reachable
  without parsing bytes during collection.
- **Live**: a blob with a holder, or a child of a live blob.
- **Grace**: the minimum age since a blob was last put before it may be
  considered dead. It covers the window between obtaining a ref and
  committing the row that holds it.
- **Sweep**: one pass that deletes dead blobs older than the grace.

## Goal

Deleting a session frees every blob only that session used, on the next sweep,
while blobs shared with a surviving session, a bot, a VFS snapshot, or a
checkpoint stay readable. Blobs nothing ever referenced, such as debug dumps
and abandoned uploads, disappear once they outlive the grace. No writer has to
remember to register what it wrote: the store derives reachability from the
rows it already maintains.

## Problem

### Roots are never recorded

`cas_session_roots`, `cas_blob_edges`, and `BlobGraphStore` were created with
the Postgres store as "reachability metadata for future garbage collection".
Nothing in production calls `record_session_blob_roots`; the only caller is
the Postgres live test. `record_blob_edges` has three callers (model job
result sets, await aggregates, environment job results) and no others. In the
local development database both tables are empty. Any collector built on the
roots table today would delete every blob.

### No deletion primitive exists

`BlobStore` offers put, put_many, read, has, and stat. None of the Postgres,
filesystem, in-memory, or cached backends can delete or enumerate blobs.
`cas_blobs` has no created or touched timestamp. Session deletion removes the
session row and lets cascades take events, checkpoints, and roots; blobs are
not mentioned. The filesystem store removes only the session directory.

### The put path has the classic content-addressed race

`put_single_blob` returns early when `has_blob` is true. Consider blob X,
written by session A, which has since been deleted:

1. A sweep marks X dead.
2. Session B produces identical content, computes the same digest, sees the
   row, and returns the ref without writing anything.
3. The sweep deletes X.
4. B's event lands referencing X, and the ref dangles.

Dedup makes a "new" write resolve to a row a sweeper is about to remove, and
neither side sees the other. Any collector must close this window.

### References live in six kinds of places

| Holder | Protection today |
| --- | --- |
| `session_events.entry_json` | none; refs are strings inside JSONB |
| `session_checkpoints.state_digest` | FK `ON DELETE RESTRICT` |
| `vfs_snapshots.digest`, `vfs_workspaces.base/head_snapshot_digest` | FK `ON DELETE RESTRICT` |
| `cas_blob_edges.child_digest` | FK `ON DELETE RESTRICT`; parent cascades |
| `bot_events.document_ref`, `prompt_ref`, `media_json` | none; plain text, no FK |
| Temporal workflow state of open sessions | none; refs not yet appended |

Several JSON blob formats embed refs to other blobs without recording edges:
VFS snapshot manifests (file blobs), native MCP outputs (`blobRef` assets),
skill catalogs (`skill_doc_ref`), prompt instruction reports (`content_ref`),
and environment projection publications (VFS snapshot routes). The FK on a
VFS snapshot protects the manifest, not the files it lists.

### Measurement

Read-only queries against the local development database on 2026-09-03
(36 sessions, 762 events, 261 blobs, all inline), extracting every
`sha256:<64 hex>` string from event JSON and joining it with `cas_blobs`:

| Category | Blobs | Bytes | Share |
| --- | ---: | ---: | ---: |
| Referenced by at least one event | 202 | 32 KB | 12% |
| Raw provider requests | 12 | 75 KB | 27% |
| Raw provider responses | 12 | 80 KB | 29% |
| Superseded checkpoint states | 3 | 70 KB | 26% |
| Current checkpoint states | 13 | 14 KB | 5% |
| Small unreferenced (acks, reports, constants) | 58 | 3 KB | 1% |

Of the referenced blobs, 136 belong to exactly one session and 66 are shared,
one by 33 sessions. The shared ones are tool schemas, engine constants, and
instructions, all small. Large per-session data is single-session except
where the retention tree already groups it: history forks read the source's
events by reference, and sub-agent result envelopes are referenced by the
parent's log.

Two findings dominate:

- **Raw provider dumps are write-only.** Every generation stores the full
  provider request and the raw response. Both refs sit on
  `LlmGenerationExecution`, and `LlmRuntime::generate_request` returns only
  the inner result, so nothing durable ever references them. Each request
  carries the whole context, so the bytes grow quadratically with turn count.
- **Superseded checkpoints leak during a session's life.** `advance_checkpoint`
  upserts the digest and abandons the previous state blob.
  [P147](p147-session-checkpoints-and-bounded-reads.md) explicitly deferred
  this to "ordinary CAS reachability collection". This item is that
  collection.

## Decision

1. **The store derives roots; writers do not register them.** Event append
   extracts every blob ref from the entry it stores and records it in
   `cas_session_roots` inside the same transaction. The writer-side
   `record_session_blob_roots` is removed from `BlobGraphStore`. A missed
   write site can no longer cause data loss because there are no write sites.
2. **Liveness is reachability, not reference counting.** A blob is live when
   a root row, an FK-protected holder, a bot event, or an edge from a live
   parent references it. Counting would have to parse JSON at delete time and
   contend on hot rows; reachability is a join over rows that already exist.
3. **Every put touches the catalog row.** `cas_blobs` gains `created_at_ms`
   and `touched_at_ms`. A put of existing content updates `touched_at_ms`
   instead of returning after a bare existence check. Bytes are never
   rewritten. Reads do not touch.
4. **A grace period, not coordination, closes the put race.** The sweep only
   considers blobs whose `touched_at_ms` is older than the grace. In the race
   above, step 2 bumps the timestamp and step 3 finds nothing to delete. The
   default grace is seven days, long enough to cover the longest activity,
   sub-agent, or environment-job window that holds a ref before appending it,
   and a human uploading through `blobs/put` before starting a run.
5. **Nested formats record edges at write time.** The writer knows why a blob
   embeds another ref and records `contains` edges there. The collector never
   infers edges from bytes, as the existing design note requires.
6. **Collection is asynchronous and iterative.** A worker-role sweeper beside
   the retention reaper deletes, per universe and per pass, a bounded batch of
   blobs with no root, no holder, no incoming edge, and an expired grace.
   Parents are collected before children: deleting a parent cascades its edge
   rows, and the children become candidates on the next pass. Session deletion
   itself never deletes blobs, because the grace would have to be waited out
   inside the transaction.
7. **Rows go before objects.** For object-backed blobs the catalog row is
   deleted first and the object second. A failure between the two leaves an
   unreadable-by-anyone object, which is a harmless leak; the reverse order
   would leave a row whose reads fail.
8. **Unrooted blobs have a bounded life.** A blob nothing references lives for
   exactly the grace. That gives debug dumps and abandoned uploads a natural
   time to live without a separate TTL mechanism.
9. **Raw provider dumps stop by default.** The generation adapters no longer
   store the provider request and raw response. A debug switch restores them
   as unrooted blobs, so enabling it costs at most the grace's worth of
   storage.
10. **Engine constants are pinned.** The well-known blobs that
    `ensure_engine_blobs` writes are excluded from every sweep by digest, so a
    long-running process never references a constant that a sweep removed.

## Reachability Model

```text
live(blob) :=
    exists cas_session_roots row for blob            -- session events
 or exists session_checkpoints.state_digest = blob   -- FK RESTRICT
 or exists vfs_snapshots.digest = blob               -- FK RESTRICT
 or exists vfs_workspaces.{base,head} = blob         -- FK RESTRICT
 or exists bot_events holding blob                   -- document, prompt, media
 or blob is pinned                                   -- engine constants
 or exists cas_blob_edges(parent, blob) with live(parent)
```

The sweep does not evaluate the recursive clause. It skips any blob with an
incoming edge and lets the cascade from a deleted parent expose the child on a
later pass. Nesting in the current formats is at most three levels deep, so a
dead subtree drains in three passes.

The invariant every subsystem must keep: a blob a session needs after a
restart is reachable from that session's own log, its checkpoint, or an
FK-protected holder. Refs held only in Temporal workflow state are covered by
the grace until they are appended. Slice 1 includes an audit of the workflow
start arguments and activity results against this invariant.

Retention-tree edge cases:

- A history fork reads the source's events by reference and holds no root
  rows for them. The source cannot be deleted while the fork exists, and a
  cascade deletes both, so the source's roots disappear exactly when nothing
  needs them.
- A sub-agent envelope is referenced by the parent's log. Parent and child are
  one retention tree.
- A config-only clone writes its own events, which re-reference the same
  content-addressed config blobs. Those roots keep the blobs alive after the
  source is deleted.

## Design

### Schema

```sql
ALTER TABLE cas_blobs ADD COLUMN IF NOT EXISTS created_at_ms bigint;
ALTER TABLE cas_blobs ADD COLUMN IF NOT EXISTS touched_at_ms bigint;
UPDATE cas_blobs SET created_at_ms = <migration time>, touched_at_ms = <migration time>
    WHERE touched_at_ms IS NULL;
ALTER TABLE cas_blobs ALTER COLUMN created_at_ms SET NOT NULL;
ALTER TABLE cas_blobs ALTER COLUMN touched_at_ms SET NOT NULL;
ALTER TABLE cas_blobs ADD CONSTRAINT cas_blobs_touched_after_created
    CHECK (touched_at_ms >= created_at_ms);
CREATE INDEX IF NOT EXISTS cas_blobs_touched_at_idx
    ON cas_blobs (universe_id, touched_at_ms);
```

Backfilling both timestamps to the migration time means the first sweep after
an upgrade happens one full grace later, and no pre-existing blob is eligible
before the roots backfill below has run.

The roots table keeps its shape. Its primary key, its cascade from `sessions`,
and its `RESTRICT` on `cas_blobs` are already what the sweep needs. The
migration backfills it from the existing logs:

```sql
INSERT INTO cas_session_roots (universe_id, session_id, digest, root_kind, first_seq, last_seq)
SELECT e.universe_id, e.session_id, r.digest, 'event', min(e.seq), max(e.seq)
FROM session_events e
CROSS JOIN LATERAL (
    SELECT DISTINCT substr(v #>> '{}', 8) AS digest
    FROM jsonb_path_query(
        e.entry_json,
        'strict $.** ? (@.type() == "string" && @ like_regex "^sha256:[0-9a-f]{64}$")'
    ) AS v
) AS r
WHERE EXISTS (SELECT 1 FROM cas_blobs b WHERE b.universe_id = e.universe_id AND b.digest = r.digest)
GROUP BY e.universe_id, e.session_id, r.digest
ON CONFLICT DO NOTHING;
```

The `EXISTS` clause skips refs whose blob is already missing. The migration
counts and logs those so an operator learns about pre-existing dangling refs
instead of the migration failing on them.

Bump `REQUIRED_SCHEMA_REVISION` and `LIGHTSPEED_SCHEMA_REVISION` to 15
together.

### Store

- `append_session_events` runs the same extraction over each appended entry
  and upserts `cas_session_roots` rows with `root_kind = 'event'` in the
  append transaction, using the existing `LEAST`/`GREATEST` sequence merge.
  The `RESTRICT` FK now makes an event that references a missing blob fail the
  append with a typed `SessionStoreError`. That is the correct failure: such an
  event was corrupt before and only failed later, at read time.
- `put_single_blob` becomes touch-or-insert. It first runs
  `UPDATE cas_blobs SET touched_at_ms = $now WHERE ... RETURNING 1`; only
  when no row exists does it upload the object and insert the row with
  `ON CONFLICT DO UPDATE SET touched_at_ms = EXCLUDED.touched_at_ms`. The
  second clause handles two writers racing on a new digest. The blob cache is
  unaffected: it caches hash-verified bytes, never liveness.
- `BlobGraphStore` loses `record_session_blob_roots` and keeps
  `record_blob_edges`. `SessionBlobRoot` becomes store-internal.
- `PgStore` gains inherent sweep methods, following `session_deletion`'s use
  of the concrete store rather than a new public trait:
  `list_sweep_candidates(cutoff_ms, limit)`, `delete_dead_blobs(digests,
  cutoff_ms)`, and `delete_blob_objects(keys)`. The delete statement repeats
  every liveness predicate and the `touched_at_ms < cutoff` guard in its
  `WHERE`, so a holder or a touch that appeared between selection and deletion
  wins. A `RESTRICT` violation on an FK holder the query forgot is reported
  as a sweep error, never retried in a loop, and counts in the pass
  statistics.
- The in-memory blob store gains the same touch and delete behavior so the
  sweeper's decision logic is unit-testable without Postgres. The filesystem
  store is unchanged.

### Edges for nested formats

Record `contains` edges at these write sites; each already has the parent
ref and the child refs in hand:

| Writer | Parent | Children |
| --- | --- | --- |
| `vfs::snapshot` manifest writes | manifest | every file blob in the manifest |
| native MCP tool activity | output blob | attached asset blobs |
| skill catalog build | catalog | `skill_doc_ref` per skill |
| prompt instructions assembler | report | `content_ref` per source |
| environment projection publication | projection snapshot | VFS snapshot refs in routes |

Model job results, await aggregates, and environment job results already
record edges. Checkpoint state embeds context entry refs but records no
edges: state is a fold over the log, so every ref in it is already rooted by
the events it was folded from. Sub-agent envelopes carry inline text, not
refs.

Each format gets a unit test that serializes a value, extracts every ref from
the bytes, and asserts the writer recorded exactly that set as edges. This is
the only place bytes are scanned for refs, and it runs in tests, not in the
collector.

### Sweeper

- Lives in `worker/reaper.rs` beside `SessionRetentionReaper`, in the
  `sessions` role, every five minutes, at most 1024 blobs per universe per
  pass. A large backlog drains over passes rather than in one transaction
  storm.
- A pass per universe: select candidates older than the cutoff, delete their
  rows in one guarded statement, commit, then delete object-store keys for
  the rows that were object-backed. Object deletion failures are logged and
  counted; the objects are unreachable and the next release of the sweeper can
  add an orphan-object listing if it ever matters.
- Structured pass statistics: universes scanned, candidates, rows deleted,
  bytes freed, objects deleted, object errors, holder conflicts, errors.
- `LIGHTSPEED_CAS_SWEEP_GRACE_MS` sets the grace, default seven days. `0`
  disables the sweeper. Documented in `docs/variables.md` next to the role
  description.
- `temporal-server -- cas-sweep [--dry-run]` runs one pass from the command
  line and prints the statistics, matching the `migrate` and `schema-version`
  subcommands. Dry run reports what a pass would delete without deleting.
- Universe deletion additionally removes every object under
  `universes/<id>/cas/` after the row cascade, so an operator dropping an
  evaluation universe does not leave its objects behind.

### Provider dumps

- `LlmGenerationExecution` loses `provider_request_ref` and
  `raw_response_ref`. The adapters stop calling `put_json` for the request
  and raw response.
- `LIGHTSPEED_LLM_DEBUG_DUMPS=true` restores both writes as unrooted blobs
  and logs their refs at debug level with the session and run id. They expire
  with the grace.
- The compaction path drops its already-unused `_provider_request_ref` and
  `_raw_response_ref` writes outright.

### API

None. The public API does not expose the sweep; `blobs/put`, `blobs/read`,
and `blobs/has` are unchanged. A blob read after collection returns the
existing not-found error.

## Acceptance

- Appending an event that references blobs writes one root row per
  `(session, digest)` with the correct sequence range; appending a second
  event widens `last_seq`. Appending an event that references a missing blob
  fails with a typed error and writes nothing.
- The migration backfills roots for every existing event ref whose blob
  exists, reports the number of dangling refs, and stamps both timestamps so
  no existing blob is eligible before one full grace.
- Putting existing content updates `touched_at_ms` and does not rewrite bytes
  or re-upload the object.
- After a session is deleted, a sweep with an expired grace removes blobs
  only that session referenced and keeps blobs any surviving session, bot
  event, checkpoint, VFS snapshot, or workspace references. A repeated sweep
  is a no-op.
- A blob touched within the grace survives a sweep even with no holder. A
  blob nothing ever referenced is collected once its grace expires.
- A child with a live parent edge survives; after the parent is collected the
  child is collected on the following pass.
- Advancing a checkpoint leaves the previous state blob collectable, and a
  state digest shared by two sessions' checkpoints survives until both move.
- Cascading deletion of a fork tree frees the source's blobs; a config-only
  clone keeps the config blobs it re-referenced after its source is deleted.
- An object-backed blob's object is removed after its row, verified against
  the local MinIO stack. The engine's constant blobs are never collected.
- The sweeper skips a universe whose deletion statement hits an FK holder,
  records a holder conflict, and continues with the next universe.
- Generation adapters store no provider request or raw response blobs unless
  the debug switch is set; with it set, the dumps are unrooted.
- Dry-run reports the same candidate count and bytes a real pass would delete.

## Non-Goals

- Reference counts on `cas_blobs`.
- Collecting in the filesystem store; it serves tests and local runs.
- Listing the object store to find objects without rows.
- Compaction or repacking of inline blobs.
- Touching on read, or any per-blob time to live independent of the grace.
- Recording edges for checkpoint state.
- Cross-universe deduplication; blobs stay universe-scoped.

## Dropped

Reviewed and rejected; do not re-propose:

- Writer-side root registration as the primary mechanism. It exists, has
  never been called, and any missed site is silent data loss.
- Inferring edges by scanning blob bytes inside the collector. Formats are
  the writer's knowledge; scanning is confined to tests.
- Deleting blobs inside the session-deletion transaction. Sharing is only
  known through the index, and the grace still has to elapse.
- Two-phase or epoch-based put coordination. A timestamp update on the
  catalog row gives the same guarantee for one indexed write.
- A `keep` flag or per-blob retention policy.
- Making bot event refs foreign keys in this item. The sweep predicate covers
  them; the bots schema can adopt FKs when it is next revised.

## Implementation Slices

### Slice 1 — Timestamps, touch-or-insert, store-derived roots, backfill

Schema revision 15, `put_single_blob` rewrite, append-time root extraction,
`record_session_blob_roots` removal, dangling-ref append error, and the
workflow-state audit against the reachability invariant.

### Slice 2 — Provider dump removal and debug switch

Independent of the rest; can ship first.

### Slice 3 — Edges for nested formats

The five writers above, each with its byte-scan equivalence test.

### Slice 4 — Sweeper, CLI pass, object deletion, universe prefix cleanup

Reaper loop, statistics, `cas-sweep --dry-run`, MinIO live test, variables
documentation.

## Implementation Notes

Recorded after implementation; the sections above are the proposal.

- **`cas_session_roots` is gone.** Derived roots made the table a
  materialization of the event rows, so migration 15 drops it and instead
  adds a stored generated column `session_events.blob_refs` (the jsonpath
  walk over the entry for exact `sha256:` strings) with a GIN
  `jsonb_path_ops` index. The sweep's first predicate is "no event of this
  universe contains the ref". Nothing is registered at append; the append
  only verifies that every embedded ref names a stored blob and fails with
  the typed error otherwise. Backfill is the column filling itself, and the
  rows cascade with the session. The `RESTRICT` guard the table's foreign key
  gave is covered by the grace, exactly as for refs held in workflow state.

- **Fingerprints looked like refs.** The dev database held 52 event refs
  whose blob did not exist, all in `turn.planned` events: the engine's LLM
  request and compaction fingerprints were formatted `sha256:<hex>` without
  ever being blobs, and the skill catalog and prompt report fingerprints did
  the same inside their documents. Under derived roots every planned turn
  would have failed to append. The engine fingerprints now carry
  `llm:sha256:` and `compaction:sha256:` prefixes (the workflow-tool
  fingerprints already used `wtx:`/`wti:`/`wtr:`), and the catalog and report
  fingerprints store the bare hex next to their `algorithm` field. The
  migration's backfill skips such strings and reports their count.
- **Clone lineage foreign key.** The acceptance case "a config-only clone
  keeps its blobs after its source is deleted" could not run: the composite
  `sessions.source_session_id` foreign key used a plain `SET NULL`, which
  nulled `universe_id` too and made deleting any session with a surviving
  clone fail. Fresh databases carried the constraint twice (auto-named from
  the `CREATE TABLE` and by name from the follow-up block). Migration 15
  drops both and re-adds one with `ON DELETE SET NULL (source_session_id)`,
  which needs PostgreSQL 15.
- **Edges cover every embedded ref, not only documents.** The five writers
  record an edge for each ref their bytes embed, including the snapshot
  manifests and workspace heads a catalog or report was read from. A reader
  following any ref in a live document finds a live blob; the cost is that a
  report keeps its source manifests alive while the report is rooted.
- **Debug dumps stay inspectable.** `LlmGenerationExecution` carries
  `debug_dumps: Option<LlmDebugDumps>`; the redaction unit tests and the
  provider live tests build their adapters with dumps enabled so the exact
  provider exchange remains checkable.
- **Sweeper pass logic is unit-tested.** The pass runs against a small
  `CasSweepStore` trait implemented by `PgStore` and, in tests, by a double
  over the in-memory blob store (which now tracks touch times, deletes, and
  records edges). Liveness itself stays a Postgres query, exercised by the
  `store-pg` live suite against Postgres and MinIO.
- **Universe purge clears the object prefix.** After the row cascade the
  operator delete lists `universes/<id>/cas/` in the object store and removes
  whatever the catalog no longer knew about.
- **Candidate selection scans live blobs too.** A pass walks a universe's
  blobs in `touched_at_ms` order and applies the liveness anti-joins until it
  has 1024 dead ones, so a universe whose old blobs are mostly live pays
  roughly one index probe per holder table per blob every pass. Fine at the
  current scale; if it ever shows up, a per-universe sweep watermark or a
  materialized dead set bounds it without changing the model.
