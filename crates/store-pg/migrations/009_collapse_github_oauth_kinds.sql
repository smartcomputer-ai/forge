-- Collapse the vestigial GitHub OAuth provider kinds into `custom_oauth` and
-- admit credentialless OpenAI-compatible model endpoint provider rows.
--
-- `github_app_user` and `github_oauth_app` never carried behaviour distinct
-- from `custom_oauth`: same OAuth client/flow code, same stored-token broker
-- source, same refresh path. GitHub identity lives in `provider_id` (and the
-- client's endpoints), not in the kind. Grant kinds grow only when the core
-- must behave differently (P127 D0); vendor labels do not qualify.
--
-- Rows are rewritten first, then every kind list is narrowed with the
-- DROP + ADD pairs the 004 migration established. `model_endpoint` belongs
-- only to auth_providers: it is transport configuration, never a grant kind.

UPDATE auth_grants   SET provider_kind = 'custom_oauth' WHERE provider_kind IN ('github_app_user', 'github_oauth_app');
UPDATE auth_clients  SET provider_kind = 'custom_oauth' WHERE provider_kind IN ('github_app_user', 'github_oauth_app');
UPDATE auth_flows    SET provider_kind = 'custom_oauth' WHERE provider_kind IN ('github_app_user', 'github_oauth_app');
UPDATE auth_providers SET provider_kind = 'custom_oauth' WHERE provider_kind IN ('github_app_user', 'github_oauth_app');

ALTER TABLE auth_grants DROP CONSTRAINT IF EXISTS auth_grants_provider_kind_known;
ALTER TABLE auth_grants ADD CONSTRAINT auth_grants_provider_kind_known
    CHECK (
        provider_kind IN (
            'static_bearer',
            'mcp_oauth',
            'github_app',
            'custom_oauth',
            'model_api_key',
            'model_oauth'
        )
    );

ALTER TABLE auth_clients DROP CONSTRAINT IF EXISTS auth_clients_provider_kind_oauth;
ALTER TABLE auth_clients ADD CONSTRAINT auth_clients_provider_kind_oauth
    CHECK (provider_kind IN ('mcp_oauth', 'custom_oauth'));

ALTER TABLE auth_flows DROP CONSTRAINT IF EXISTS auth_flows_provider_kind_oauth;
ALTER TABLE auth_flows ADD CONSTRAINT auth_flows_provider_kind_oauth
    CHECK (provider_kind IN ('mcp_oauth', 'custom_oauth'));

ALTER TABLE auth_providers DROP CONSTRAINT IF EXISTS auth_providers_provider_kind_known;
ALTER TABLE auth_providers ADD CONSTRAINT auth_providers_provider_kind_known
    CHECK (
        provider_kind IN (
            'static_bearer',
            'mcp_oauth',
            'github_app',
            'custom_oauth',
            'model_api_key',
            'model_oauth',
            'model_endpoint'
        )
    );
