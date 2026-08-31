-- P143 makes current Streamable HTTP the sole remote MCP transport. Transport
-- remains an internal catalog extension point, but legacy SSE and automatic
-- transport selection are not supported by the greenfield public product.

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
