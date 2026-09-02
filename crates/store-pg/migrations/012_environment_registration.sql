-- Key-based outbound environment registration: reusable, universe-scoped
-- registration keys and the registered environment source they admit.
--
-- Design notes:
-- - A registration key is an admission policy and the group of the
--   environments it admitted. It stores no counters: registration and active
--   counts derive from environment rows carrying registration_key_id.
-- - Only the SHA-256 hash of the server-generated secret is stored, exactly
--   like api_keys. key_prefix is the display handle.
-- - identity_mode is key policy copied onto every environment the key
--   admits; the daemon neither knows nor chooses it.
-- - The daemon public key is the identity. One public key maps to at most
--   one environment in the whole deployment, ever: the unique index covers
--   closed rows too, so a closed environment's daemon cannot register again
--   without a fresh local identity.
-- - last_seen_at_ms is the gateway's heartbeat stamp on the control
--   connection; the lifecycle reconciler derives ephemeral cleanup and stale
--   Ready repair from it.

CREATE TABLE IF NOT EXISTS environment_registration_keys (
    universe_id uuid NOT NULL REFERENCES universes (universe_id) ON DELETE CASCADE,
    registration_key_id text NOT NULL,
    display_name text NOT NULL,
    key_prefix text NOT NULL,
    secret_hash text NOT NULL,
    identity_mode text NOT NULL,
    max_active_environments integer,
    ephemeral_disconnect_grace_ms bigint,
    expires_at_ms bigint,
    created_at_ms bigint NOT NULL,
    revoked_at_ms bigint,
    PRIMARY KEY (universe_id, registration_key_id),
    CONSTRAINT environment_registration_keys_id_format
        CHECK (registration_key_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT environment_registration_keys_display_name_bounded
        CHECK (display_name <> '' AND length(display_name) <= 128),
    CONSTRAINT environment_registration_keys_key_prefix_not_empty
        CHECK (key_prefix <> ''),
    CONSTRAINT environment_registration_keys_secret_hash_format
        CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT environment_registration_keys_identity_mode_known
        CHECK (identity_mode IN ('persistent', 'ephemeral')),
    CONSTRAINT environment_registration_keys_limits_positive CHECK (
        (max_active_environments IS NULL OR max_active_environments > 0)
        AND (ephemeral_disconnect_grace_ms IS NULL OR ephemeral_disconnect_grace_ms > 0)
    ),
    CONSTRAINT environment_registration_keys_times_valid CHECK (
        created_at_ms >= 0
        AND (expires_at_ms IS NULL OR expires_at_ms >= 0)
        AND (revoked_at_ms IS NULL OR revoked_at_ms >= created_at_ms)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS environment_registration_keys_secret_hash_idx
    ON environment_registration_keys (secret_hash);

CREATE UNIQUE INDEX IF NOT EXISTS environment_registration_keys_key_prefix_idx
    ON environment_registration_keys (key_prefix);

ALTER TABLE environments
    ADD COLUMN IF NOT EXISTS registration_key_id text,
    ADD COLUMN IF NOT EXISTS daemon_id text,
    ADD COLUMN IF NOT EXISTS daemon_public_key text,
    ADD COLUMN IF NOT EXISTS identity_mode text,
    ADD COLUMN IF NOT EXISTS last_seen_at_ms bigint;

ALTER TABLE environments DROP CONSTRAINT IF EXISTS environments_source_known;
ALTER TABLE environments ADD CONSTRAINT environments_source_known
    CHECK (source_kind IN ('provisioned', 'external', 'registered'));

ALTER TABLE environments DROP CONSTRAINT IF EXISTS environments_source_fields;
ALTER TABLE environments ADD CONSTRAINT environments_source_fields CHECK (
    (
        source_kind = 'provisioned'
        AND provider_id IS NOT NULL AND binding_id IS NOT NULL
        AND daemon_connection_json IS NULL
        AND registration_key_id IS NULL AND daemon_id IS NULL
        AND daemon_public_key IS NULL AND identity_mode IS NULL
        AND last_seen_at_ms IS NULL
    )
    OR (
        source_kind = 'external'
        AND provider_id IS NULL AND binding_id IS NULL
        AND jsonb_typeof(daemon_connection_json) = 'object'
        AND registration_key_id IS NULL AND daemon_id IS NULL
        AND daemon_public_key IS NULL AND identity_mode IS NULL
        AND last_seen_at_ms IS NULL
    )
    OR (
        source_kind = 'registered'
        AND provider_id IS NULL AND binding_id IS NULL
        AND daemon_connection_json IS NULL
        AND registration_key_id IS NOT NULL
        AND daemon_id ~ '^daemon_[0-9a-f]{64}$'
        AND daemon_public_key ~ '^[0-9a-f]{64}$'
        AND identity_mode IN ('persistent', 'ephemeral')
        AND (last_seen_at_ms IS NULL OR last_seen_at_ms >= 0)
    )
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'environments_registration_key_fk'
    ) THEN
        ALTER TABLE environments ADD CONSTRAINT environments_registration_key_fk
            FOREIGN KEY (universe_id, registration_key_id)
            REFERENCES environment_registration_keys (universe_id, registration_key_id)
            ON DELETE RESTRICT;
    END IF;
END $$;

-- Deployment-wide: one daemon identity, one environment, ever.
CREATE UNIQUE INDEX IF NOT EXISTS environments_daemon_public_key_idx
    ON environments (daemon_public_key)
    WHERE daemon_public_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS environments_registration_key_idx
    ON environments (universe_id, registration_key_id, status)
    WHERE registration_key_id IS NOT NULL;

COMMENT ON TABLE environment_registration_keys IS
    'Reusable universe-scoped admission policies for outbound envd registration; each key is also the group of the environments it admitted.';
COMMENT ON COLUMN environments.daemon_public_key IS
    'Lowercase hex Ed25519 public key of the registered daemon; the identity, unique across the deployment including closed rows.';
COMMENT ON COLUMN environments.last_seen_at_ms IS
    'Gateway heartbeat stamp on the registered control connection; stale under ready means the gateway stopped without recording the disconnect.';
