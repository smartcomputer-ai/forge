-- Current remote MCP transport, rmcp OAuth protocol state, approvals, and
-- native MCP execution.
--
-- Streamable HTTP is the sole public transport. Transport remains an internal
-- catalog extension point, but legacy SSE and automatic transport selection
-- are not supported by the greenfield product.

UPDATE mcp_servers
SET transport = 'streamable_http'
WHERE transport <> 'streamable_http';

ALTER TABLE mcp_servers
    ALTER COLUMN transport SET DEFAULT 'streamable_http';

ALTER TABLE mcp_servers
    DROP CONSTRAINT mcp_servers_transport_known;

ALTER TABLE mcp_servers
    ADD CONSTRAINT mcp_servers_transport_known
        CHECK (transport IN ('streamable_http'));

-- Durable rmcp OAuth protocol outputs. Tokens, client secrets, PKCE
-- verifiers, and raw callback state remain in encrypted secret storage or
-- outside persistence. These columns retain only the public issuer/scope facts
-- needed to reconstruct an authorization on a different gateway process.

ALTER TABLE auth_clients
    ADD COLUMN IF NOT EXISTS authorization_server_issuer text,
    ADD COLUMN IF NOT EXISTS authorization_response_iss_parameter_supported boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS authorization_server_scopes_supported text[] NOT NULL DEFAULT '{}';

ALTER TABLE auth_clients
    ADD CONSTRAINT auth_clients_authorization_server_issuer_not_empty
    CHECK (authorization_server_issuer IS NULL OR authorization_server_issuer <> '');

ALTER TABLE auth_flows
    ADD COLUMN IF NOT EXISTS expected_issuer text,
    ADD COLUMN IF NOT EXISTS require_issuer boolean NOT NULL DEFAULT false;

ALTER TABLE auth_flows
    ADD CONSTRAINT auth_flows_expected_issuer_not_empty
    CHECK (expected_issuer IS NULL OR expected_issuer <> '');

COMMENT ON COLUMN auth_clients.authorization_server_issuer IS
    'Public authorization-server issuer selected by rmcp discovery; never a credential.';
COMMENT ON COLUMN auth_flows.expected_issuer IS
    'Issuer frozen when the authorization request was created for RFC 9207 callback validation.';

UPDATE mcp_servers
SET approval_default = 'never'
WHERE approval_default = 'provider_default';

ALTER TABLE mcp_servers
    ALTER COLUMN approval_default SET DEFAULT 'never';

ALTER TABLE mcp_servers
    DROP CONSTRAINT mcp_servers_approval_default_known;

ALTER TABLE mcp_servers
    ADD CONSTRAINT mcp_servers_approval_default_known
        CHECK (approval_default IN ('always', 'never'));

ALTER TABLE mcp_servers
    ADD COLUMN execution TEXT NOT NULL DEFAULT 'provider',
    ADD COLUMN exposure TEXT NOT NULL DEFAULT 'inject',
    ADD COLUMN allow_private_network BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE mcp_servers
    ADD CONSTRAINT mcp_servers_execution_known
        CHECK (execution IN ('provider', 'native')),
    ADD CONSTRAINT mcp_servers_exposure_known
        CHECK (exposure IN ('inject', 'search')),
    ADD CONSTRAINT mcp_servers_exposure_matches_execution
        CHECK (execution = 'native' OR exposure = 'inject');
