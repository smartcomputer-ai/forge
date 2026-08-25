-- Sub-agent lineage (P134).
--
-- Design notes:
-- - A delegated child session carries typed provenance on its own row as one
--   document (`origin_json`, the serialized `SessionOrigin`: parent, parent
--   run, root, depth, invocation, pinned profile revision, effective limits).
--   Provenance, never ownership: the child is an ordinary session.
-- - Only two facts are ever queried, so only they are denormalized into
--   indexed columns: the root (root-scoped limit counts, `rootSessionId`
--   list filter) and the parent (`parentSessionId` list filter).
-- - The child row is the root-scoped reservation. Stores lock the root row,
--   count descendants by origin_root_session_id, and insert in one
--   transaction, so limits hold under concurrent spawns.
-- - session_links (the fleet graph) is dropped; nothing else wrote it.
-- - No foreign keys to the parent/root: a child survives its ancestors'
--   deletion as history; the origin ids stay as facts.

DROP TABLE IF EXISTS session_links;

ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS origin_json jsonb,
    ADD COLUMN IF NOT EXISTS origin_root_session_id text,
    ADD COLUMN IF NOT EXISTS origin_parent_session_id text;

ALTER TABLE sessions
    DROP CONSTRAINT IF EXISTS sessions_origin_shape;
ALTER TABLE sessions
    ADD CONSTRAINT sessions_origin_shape CHECK (
        (
            origin_json IS NULL
            AND origin_root_session_id IS NULL
            AND origin_parent_session_id IS NULL
        )
        OR (
            jsonb_typeof(origin_json) = 'object'
            AND origin_root_session_id IS NOT NULL
            AND origin_root_session_id <> ''
            AND origin_parent_session_id IS NOT NULL
            AND origin_parent_session_id <> ''
        )
    );

CREATE INDEX IF NOT EXISTS sessions_origin_root_idx
    ON sessions (universe_id, origin_root_session_id)
    WHERE origin_root_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS sessions_origin_parent_idx
    ON sessions (universe_id, origin_parent_session_id)
    WHERE origin_parent_session_id IS NOT NULL;

COMMENT ON COLUMN sessions.origin_json IS
    'Serialized SessionOrigin of a sub-agent child (kind, parent, parent run, root, depth, invocation, pinned profile revision, effective limits); null for roots.';
COMMENT ON COLUMN sessions.origin_root_session_id IS
    'Denormalized from origin_json: the lineage root; root-scoped limits count every session naming this root.';
COMMENT ON COLUMN sessions.origin_parent_session_id IS
    'Denormalized from origin_json: the delegating parent session.';
