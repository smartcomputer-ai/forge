-- Coding-agent subscription credentials (P127 S1/S2).
--
-- One new auth grant kind, `openai_chatgpt`: a ChatGPT Enterprise access
-- token, or a Plus/Pro token set that the worker injects as Codex `auth.json`
-- content. Claude Code `setup-token` credentials are ordinary `static_bearer`
-- grants (`provider_id = 'anthropic'`, `metadata.subscription = 'claudeCode'`).
-- Grant rows and secrets keep their shape; token material stays in
-- auth_secrets. Environment credential bindings are unchanged: the grant kind
-- decides the injected value.

ALTER TABLE auth_grants DROP CONSTRAINT IF EXISTS auth_grants_provider_kind_known;
ALTER TABLE auth_grants ADD CONSTRAINT auth_grants_provider_kind_known
    CHECK (
        provider_kind IN (
            'static_bearer',
            'mcp_oauth',
            'github_app',
            'github_app_user',
            'github_oauth_app',
            'custom_oauth',
            'model_api_key',
            'model_oauth',
            'openai_chatgpt'
        )
    );
