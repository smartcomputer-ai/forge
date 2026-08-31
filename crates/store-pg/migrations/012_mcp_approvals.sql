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
