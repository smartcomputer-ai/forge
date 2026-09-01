-- Disposable, advance-only pointers to CAS-backed session reducer state.
-- The append-only event log remains the sole durable authority.

CREATE TABLE IF NOT EXISTS session_checkpoints (
    universe_id uuid NOT NULL,
    session_id text NOT NULL,
    through_seq bigint NOT NULL,
    format_version integer NOT NULL,
    state_digest text NOT NULL,
    lineage_source_session_id text,
    lineage_source_seq bigint,
    byte_len bigint NOT NULL,
    created_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, session_id),
    FOREIGN KEY (universe_id, session_id)
        REFERENCES sessions (universe_id, session_id) ON DELETE CASCADE,
    FOREIGN KEY (universe_id, state_digest)
        REFERENCES cas_blobs (universe_id, digest) ON DELETE RESTRICT,

    CONSTRAINT session_checkpoints_through_seq_positive CHECK (through_seq > 0),
    CONSTRAINT session_checkpoints_format_version_positive CHECK (format_version > 0),
    CONSTRAINT session_checkpoints_byte_len_nonnegative CHECK (byte_len >= 0),
    CONSTRAINT session_checkpoints_created_at_ms_nonnegative CHECK (created_at_ms >= 0),
    -- Clones record a source session with no source sequence (fresh log);
    -- only a sequence without a session is malformed.
    CONSTRAINT session_checkpoints_lineage_pair CHECK (
        NOT (lineage_source_session_id IS NULL AND lineage_source_seq IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS session_checkpoints_state_digest_idx
    ON session_checkpoints (universe_id, state_digest);

COMMENT ON TABLE session_checkpoints IS
    'Disposable advance-only pointers to CAS-backed reducer state; session_events remains authoritative.';
