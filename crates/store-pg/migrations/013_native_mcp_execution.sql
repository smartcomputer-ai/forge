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
