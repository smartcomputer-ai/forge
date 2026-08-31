-- P143b: durable rmcp OAuth protocol outputs.
--
-- Tokens, client secrets, PKCE verifiers, and raw callback state remain in
-- encrypted secret storage or outside persistence. These columns retain only
-- the public issuer/scope facts needed to reconstruct an authorization on a
-- different gateway process.

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
