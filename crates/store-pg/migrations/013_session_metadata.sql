-- Descriptive session metadata: a bounded string map validated at the API
-- boundary, stored on the session row (never in the event log) and filtered
-- by jsonb containment. Registered environments carry the same map; both
-- tables get the GIN index the containment filter uses.

ALTER TABLE sessions
    ADD COLUMN IF NOT EXISTS metadata_json jsonb NOT NULL DEFAULT '{}';

ALTER TABLE sessions
    DROP CONSTRAINT IF EXISTS sessions_metadata_object;

ALTER TABLE sessions
    ADD CONSTRAINT sessions_metadata_object
        CHECK (jsonb_typeof(metadata_json) = 'object');

CREATE INDEX IF NOT EXISTS sessions_metadata_idx
    ON sessions USING gin (metadata_json jsonb_path_ops);

CREATE INDEX IF NOT EXISTS environments_metadata_idx
    ON environments USING gin (metadata_json jsonb_path_ops);

COMMENT ON COLUMN sessions.metadata_json IS
    'Descriptive key/value metadata; never routing, authority, or selection.';
