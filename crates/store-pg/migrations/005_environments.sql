-- Universe environment providers, environments, and credentials.

CREATE TABLE IF NOT EXISTS environment_providers (
    universe_id uuid NOT NULL
        REFERENCES universes (universe_id) ON DELETE CASCADE,
    provider_id text NOT NULL,
    provider_kind text NOT NULL,
    display_name text,
    status text NOT NULL,
    controller_connection_json jsonb NOT NULL,
    capabilities_json jsonb NOT NULL,
    implementation_json jsonb NOT NULL,
    last_seen_ms bigint NOT NULL,
    lease_expires_ms bigint NOT NULL,
    metadata_json jsonb NOT NULL DEFAULT '{}',
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, provider_id),
    CONSTRAINT environment_providers_provider_id_format
        CHECK (provider_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT environment_providers_kind_known
        CHECK (provider_kind IN ('sandbox', 'bridge', 'custom')),
    CONSTRAINT environment_providers_status_known
        CHECK (status IN ('online', 'offline')),
    CONSTRAINT environment_providers_controller_connection_object
        CHECK (jsonb_typeof(controller_connection_json) = 'object'),
    CONSTRAINT environment_providers_capabilities_object
        CHECK (jsonb_typeof(capabilities_json) = 'object'),
    CONSTRAINT environment_providers_implementation_object
        CHECK (jsonb_typeof(implementation_json) = 'object'),
    CONSTRAINT environment_providers_metadata_object
        CHECK (jsonb_typeof(metadata_json) = 'object'),
    CONSTRAINT environment_providers_times_valid
        CHECK (
            last_seen_ms >= 0
            AND lease_expires_ms >= last_seen_ms
            AND created_at_ms >= 0
            AND updated_at_ms >= created_at_ms
        )
);

CREATE INDEX IF NOT EXISTS environment_providers_status_idx
    ON environment_providers (universe_id, status, provider_id);

CREATE TABLE IF NOT EXISTS environments (
    universe_id uuid NOT NULL,
    environment_id text NOT NULL,
    provider_id text NOT NULL,
    provider_target_id text NOT NULL,
    origin text NOT NULL,
    display_name text,
    status text NOT NULL,
    scope_json jsonb NOT NULL,
    capabilities_json jsonb NOT NULL,
    connection_json jsonb NOT NULL,
    default_cwd text,
    metadata_json jsonb NOT NULL DEFAULT '{}',
    observed_at_ms bigint NOT NULL,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, environment_id),
    UNIQUE (universe_id, provider_id, provider_target_id),
    FOREIGN KEY (universe_id, provider_id)
        REFERENCES environment_providers (universe_id, provider_id)
        ON DELETE RESTRICT,
    CONSTRAINT environments_environment_id_format
        CHECK (environment_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT environments_provider_target_id_format
        CHECK (provider_target_id ~ '^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$'),
    CONSTRAINT environments_origin_known
        CHECK (origin IN ('provided', 'provisioned')),
    CONSTRAINT environments_status_known
        CHECK (status IN ('creating', 'starting', 'ready', 'stopped', 'closing', 'closed', 'failed', 'unknown')),
    CONSTRAINT environments_scope_object CHECK (jsonb_typeof(scope_json) = 'object'),
    CONSTRAINT environments_capabilities_object CHECK (jsonb_typeof(capabilities_json) = 'object'),
    CONSTRAINT environments_connection_object CHECK (jsonb_typeof(connection_json) = 'object'),
    CONSTRAINT environments_metadata_object CHECK (jsonb_typeof(metadata_json) = 'object'),
    CONSTRAINT environments_times_valid
        CHECK (observed_at_ms >= 0 AND created_at_ms >= 0 AND updated_at_ms >= created_at_ms)
);

CREATE INDEX IF NOT EXISTS environments_provider_status_idx
    ON environments (universe_id, provider_id, status, environment_id);

CREATE TABLE IF NOT EXISTS environment_credentials (
    universe_id uuid NOT NULL,
    environment_id text NOT NULL,
    env_name text NOT NULL,
    source_kind text NOT NULL,
    grant_id text,
    auth_provider_id text,
    secret_id text,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,

    PRIMARY KEY (universe_id, environment_id, env_name),
    FOREIGN KEY (universe_id, environment_id)
        REFERENCES environments (universe_id, environment_id)
        ON DELETE CASCADE,
    FOREIGN KEY (universe_id, grant_id)
        REFERENCES auth_grants (universe_id, grant_id) ON DELETE RESTRICT,
    FOREIGN KEY (universe_id, auth_provider_id)
        REFERENCES auth_providers (universe_id, provider_id) ON DELETE RESTRICT,
    FOREIGN KEY (universe_id, secret_id)
        REFERENCES auth_secrets (universe_id, secret_id) ON DELETE RESTRICT,
    CONSTRAINT environment_credentials_env_name_format
        CHECK (env_name ~ '^[A-Za-z_][A-Za-z0-9_]{0,127}$'),
    CONSTRAINT environment_credentials_source_kind_known
        CHECK (source_kind IN ('auth_grant', 'auth_provider_credential', 'direct_secret')),
    CONSTRAINT environment_credentials_source_exactly_one CHECK (
        (source_kind = 'auth_grant' AND grant_id IS NOT NULL AND auth_provider_id IS NULL AND secret_id IS NULL)
        OR (source_kind = 'auth_provider_credential' AND grant_id IS NULL AND auth_provider_id IS NOT NULL AND secret_id IS NULL)
        OR (source_kind = 'direct_secret' AND grant_id IS NULL AND auth_provider_id IS NULL AND secret_id IS NOT NULL)
    ),
    CONSTRAINT environment_credentials_times_valid
        CHECK (created_at_ms >= 0 AND updated_at_ms >= created_at_ms)
);

-- P104: provider-owned jobs no longer have a Lightspeed registry. These
-- drops also clean up deployments that applied the pre-P104 schema.
DROP TABLE IF EXISTS environment_jobs;
DROP TABLE IF EXISTS environment_job_groups;

COMMENT ON TABLE environment_providers IS
    'Universe-scoped liveness leases for environment provider controllers.';
COMMENT ON TABLE environments IS
    'Universe-owned environments; the current provider connection source of truth.';
COMMENT ON TABLE environment_credentials IS
    'Universe-owned credential bindings for an environment.';
