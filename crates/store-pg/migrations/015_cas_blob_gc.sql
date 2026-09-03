-- Reachability metadata for content-addressed blob collection.
--
-- Design notes:
-- - Every put stamps `touched_at_ms`; a put of existing content updates it
--   instead of returning after a bare existence check. Collection only
--   considers blobs untouched for longer than a grace period, which closes
--   the window between obtaining a ref and committing the row that holds it.
-- - A session event keeps every blob it embeds alive. The refs are a stored
--   generated column on the event row itself, so the catalog needs no second
--   table, no writer registration, and no backfill: the column fills itself,
--   and the rows cascade with the session. A ref is any JSON string in the
--   stored entry that is exactly `sha256:<64 lowercase hex>`.
-- - Both timestamps backfill to the migration time, so no pre-existing blob
--   is eligible for collection before one full grace period has elapsed.

ALTER TABLE cas_blobs
    ADD COLUMN IF NOT EXISTS created_at_ms bigint,
    ADD COLUMN IF NOT EXISTS touched_at_ms bigint;

UPDATE cas_blobs
SET created_at_ms = (extract(epoch FROM now()) * 1000)::bigint,
    touched_at_ms = (extract(epoch FROM now()) * 1000)::bigint
WHERE touched_at_ms IS NULL OR created_at_ms IS NULL;

ALTER TABLE cas_blobs
    ALTER COLUMN created_at_ms SET NOT NULL,
    ALTER COLUMN touched_at_ms SET NOT NULL;

ALTER TABLE cas_blobs DROP CONSTRAINT IF EXISTS cas_blobs_created_at_ms_nonnegative;
ALTER TABLE cas_blobs ADD CONSTRAINT cas_blobs_created_at_ms_nonnegative
    CHECK (created_at_ms >= 0);
ALTER TABLE cas_blobs DROP CONSTRAINT IF EXISTS cas_blobs_touched_after_created;
ALTER TABLE cas_blobs ADD CONSTRAINT cas_blobs_touched_after_created
    CHECK (touched_at_ms >= created_at_ms);

-- The sweep scans the oldest untouched blobs of one universe.
CREATE INDEX IF NOT EXISTS cas_blobs_touched_at_idx
    ON cas_blobs (universe_id, touched_at_ms);

-- The refs an event embeds, as a JSON array of `sha256:` strings. The
-- jsonpath walk is immutable, so Postgres maintains the column on insert and
-- fills it for existing rows here. The sweep asks "does any event of this
-- universe contain this ref" through the containment index.
ALTER TABLE session_events
    ADD COLUMN IF NOT EXISTS blob_refs jsonb GENERATED ALWAYS AS (
        jsonb_path_query_array(
            entry_json,
            'strict $.** ? (@.type() == "string" && @ like_regex "^sha256:[0-9a-f]{64}$")'
        )
    ) STORED;

CREATE INDEX IF NOT EXISTS session_events_blob_refs_idx
    ON session_events USING gin (blob_refs jsonb_path_ops);

-- Writer-side root registration never had a production caller; the event
-- rows themselves are the roots now.
DROP TABLE IF EXISTS cas_session_roots;

-- Refs whose blob does not exist were dangling before this migration. They
-- are reported so an operator learns about them; from now on an append that
-- embeds such a ref is rejected.
DO $$
DECLARE
    dangling bigint;
BEGIN
    SELECT count(*) INTO dangling
    FROM (
        SELECT DISTINCT e.universe_id, e.session_id, r.blob_ref
        FROM session_events AS e
        CROSS JOIN LATERAL jsonb_array_elements_text(e.blob_refs) AS r(blob_ref)
        WHERE NOT EXISTS (
            SELECT 1 FROM cas_blobs AS b
            WHERE b.universe_id = e.universe_id AND b.blob_ref = r.blob_ref
        )
    ) AS missing;
    IF dangling > 0 THEN
        RAISE WARNING 'session events embed % blob ref(s) whose blob is missing; such appends are rejected from now on', dangling;
    ELSE
        RAISE NOTICE 'session events embed no dangling blob refs';
    END IF;
END
$$;

COMMENT ON COLUMN cas_blobs.created_at_ms IS
    'Unix milliseconds of the first put of this content in the universe.';
COMMENT ON COLUMN cas_blobs.touched_at_ms IS
    'Unix milliseconds of the most recent put of this content; the collection grace period counts from here.';
COMMENT ON COLUMN session_events.blob_refs IS
    'Generated: every sha256 blob ref string the stored entry embeds; the blobs it names are live while the row exists.';

-- A config-only clone outlives its source. The original composite foreign
-- key nulled both columns on source deletion, which violated the NOT NULL on
-- `universe_id` and made deleting any session with a surviving clone fail.
-- Fresh databases carry the constraint twice (once auto-named from the
-- CREATE TABLE, once by name from the follow-up block); drop both and keep
-- one that nulls only the lineage column (column-scoped SET NULL needs
-- PostgreSQL 15).
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_universe_id_source_session_id_fkey;
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_source_session_id_fkey;
ALTER TABLE sessions
    ADD CONSTRAINT sessions_source_session_id_fkey
    FOREIGN KEY (universe_id, source_session_id)
    REFERENCES sessions (universe_id, session_id)
    ON DELETE SET NULL (source_session_id);
