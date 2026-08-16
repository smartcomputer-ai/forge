-- Session provenance for profile-provisioned environments (P125).
--
-- Design notes:
-- - origin_session_id records which session a profile provisioned the
--   environment for. It is provenance and an optional close trigger, not
--   ownership: the environment stays a universe resource that any session may
--   select and the universe may close.
-- - origin_close_with_session asks the lifecycle reconciler to close the
--   environment once the origin session's lifecycle projection is closed.
-- - No foreign key to sessions: the environment must survive session deletion
--   until its own close completes, and the sweep tolerates a missing session
--   row by treating it as closed.

ALTER TABLE environments
    ADD COLUMN IF NOT EXISTS origin_session_id text,
    ADD COLUMN IF NOT EXISTS origin_profile_id text,
    ADD COLUMN IF NOT EXISTS origin_close_with_session boolean NOT NULL DEFAULT false;

ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_origin_session_shape;
ALTER TABLE environments
    ADD CONSTRAINT environments_origin_session_shape CHECK (
        (origin_session_id IS NULL AND origin_profile_id IS NULL AND origin_close_with_session = false)
        OR (origin_session_id IS NOT NULL AND origin_session_id <> '')
    );

CREATE INDEX IF NOT EXISTS environments_origin_session_idx
    ON environments (universe_id, origin_session_id)
    WHERE origin_session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS environments_close_with_session_idx
    ON environments (universe_id)
    WHERE origin_close_with_session = true AND status NOT IN ('closing', 'closed');

-- Binding deletion is guarded by the store ("no non-closed environment may
-- reference the binding", P118); closed environments are history and must not
-- pin the binding forever. The original RESTRICT foreign key contradicted that
-- rule, so it is dropped: closed rows keep their binding_id as a historical
-- fact without referential enforcement.
ALTER TABLE environments
    DROP CONSTRAINT IF EXISTS environments_universe_id_binding_id_fkey;
