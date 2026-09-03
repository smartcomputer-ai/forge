ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS closed_at_ms bigint,
    ADD COLUMN IF NOT EXISTS retention_root_session_id text,
    ADD COLUMN IF NOT EXISTS delete_after_close_ms bigint,
    ADD COLUMN IF NOT EXISTS delete_at_ms bigint;

UPDATE sessions
SET closed_at_ms = updated_at_ms
WHERE closed_at_seq IS NOT NULL
  AND closed_at_ms IS NULL;

-- Resolve existing ownership trees. History forks and delegated children
-- follow their parent; config-only clones are roots. Legacy delegated rows
-- whose historical parent was already deleted safely become independent.
WITH RECURSIVE retention_tree AS (
    SELECT
        session.universe_id,
        session.session_id,
        session.session_id AS root_session_id
    FROM sessions AS session
    WHERE NOT EXISTS (
        SELECT 1
        FROM sessions AS parent
        WHERE parent.universe_id = session.universe_id
          AND (
              (session.source_seq IS NOT NULL
                  AND session.source_session_id = parent.session_id)
              OR session.origin_parent_session_id = parent.session_id
          )
    )

    UNION

    SELECT
        child.universe_id,
        child.session_id,
        parent.root_session_id
    FROM retention_tree AS parent
    JOIN sessions AS child
      ON child.universe_id = parent.universe_id
     AND (
         (child.source_seq IS NOT NULL
             AND child.source_session_id = parent.session_id)
         OR child.origin_parent_session_id = parent.session_id
     )
)
UPDATE sessions AS session
SET retention_root_session_id = retention_tree.root_session_id
FROM retention_tree
WHERE session.universe_id = retention_tree.universe_id
  AND session.session_id = retention_tree.session_id;

UPDATE sessions
SET retention_root_session_id = session_id
WHERE retention_root_session_id IS NULL;

ALTER TABLE sessions
    ALTER COLUMN retention_root_session_id SET NOT NULL;

ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_closed_at_pair;
ALTER TABLE sessions ADD CONSTRAINT sessions_closed_at_pair
    CHECK ((closed_at_ms IS NULL) = (closed_at_seq IS NULL));
ALTER TABLE sessions ADD CONSTRAINT sessions_closed_at_ms_nonnegative
    CHECK (closed_at_ms IS NULL OR closed_at_ms >= 0);
ALTER TABLE sessions ADD CONSTRAINT sessions_retention_root_format
    CHECK (retention_root_session_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$');
ALTER TABLE sessions ADD CONSTRAINT sessions_delete_after_close_positive
    CHECK (delete_after_close_ms IS NULL OR delete_after_close_ms > 0);
ALTER TABLE sessions ADD CONSTRAINT sessions_delete_at_nonnegative
    CHECK (delete_at_ms IS NULL OR delete_at_ms >= 0);
ALTER TABLE sessions ADD CONSTRAINT sessions_retention_policy_on_root
    CHECK (
        retention_root_session_id = session_id
        OR (delete_after_close_ms IS NULL AND delete_at_ms IS NULL)
    );
ALTER TABLE sessions ADD CONSTRAINT sessions_retention_deadline_shape
    CHECK (
        (delete_at_ms IS NOT NULL) = (
            retention_root_session_id = session_id
            AND closed_at_ms IS NOT NULL
            AND delete_after_close_ms IS NOT NULL
        )
    );

CREATE INDEX IF NOT EXISTS sessions_retention_root_idx
    ON sessions (universe_id, retention_root_session_id, session_id);
CREATE INDEX IF NOT EXISTS sessions_retention_due_idx
    ON sessions (universe_id, delete_at_ms, session_id)
    WHERE lifecycle_status = 'closed' AND delete_at_ms IS NOT NULL;
