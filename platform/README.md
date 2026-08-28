# Lightspeed platform

The first-party Lightspeed management plane, web UI, channel workers, and
supporting TypeScript packages. The repository root is the npm workspace root;
run Node commands from there.

## Components

- `server/` — Hono API, better-auth integration, universe-scoped gateway
  passthrough, database migration startup, and static SPA hosting.
- `web/` — Vite/React management UI served under `/app`.
- `cli/` — `lightspeed-platform`, the platform administration CLI.
- `shared/` — Zod input schemas and deterministic helpers shared by the server,
  web UI, and CLI.
- `db/` — Drizzle schemas, migrations, and the platform database adapter.
- `bots/` — durable bot controllers, activities, triggers, and federation.
- `channels/` — Temporal-managed Telegram and optional WhatsApp channel roles.
- `workers/` — the shared Channels/Bots/connector runtime role dispatcher.
- `configurator-mcp/` — generated Streamable HTTP MCP facade over the
  universe-scoped Lightspeed API.
- `scripts/` — product-identity check and the generated profile configuration
  reference.
- `web/src/demo/` — the in-browser demo backend: an in-memory stand-in for
  the platform server and engine that the demo build loads instead of a
  real API (see "Demo build" below).

The generated public API client lives separately at `clients/typescript/`.
Committed wire artifacts are owned by `crates/api/contract/`.
The repository-level Docker Compose development environment lives under
`scripts/dev/`.

The authoritative configuration reference is
[`docs/variables.md`](../docs/variables.md), with separate sections for the
Platform server, Platform workers, Configurator MCP, and development-only
settings.

## Development

Install all Node workspace dependencies and run the complete check:

```bash
npm install
npm run check
```

For the complete interactive Lightspeed development stack:

```bash
./dev.sh
```

That command uses the unified supervisor under `scripts/dev/` and starts the complete
product. For the focused Platform loop against an already running runtime at
`LIGHTSPEED_API_URL`, use:

```bash
./dev.sh platform
```

The focused profile starts shared infrastructure, the Platform server on port
3000, and Vite on port 5173.

### Demo build

The web UI has two build paths. `npm run build:web` produces the live SPA that
the Platform server hosts under `/app`. `npm run build:demo` produces
`platform/web/dist-demo/`: the same SPA with `web/src/demo/main.ts` as its
entry, which installs an in-browser backend (a Hono router behind a `fetch`
shim, seeded from `web/src/demo/fixtures/`) before loading the app. It needs no
server, no sign-in, and no network — the visitor is a platform admin who owns a
few pre-populated universes, each showing a different use-case — so it can be
published as a static site (serve `dist-demo/` under `/app/` with an
`index.html` fallback for client routes).

```bash
./dev.sh demo         # Vite dev server on http://localhost:5175/app/ (alias: npm run demo)
npm run build:demo    # static site in platform/web/dist-demo/
```

The demo is also the frontend-only development loop: it is the only mock
backend in the repository, so a new API route needs a stub under
`web/src/demo/routes/` (an unstubbed route answers 404 with a `demo:` message
so the gap is visible in the UI) and demo content belongs in a fixture module.

Development defaults use `admin@lightspeed.dev` and
`lightspeed-dev-password`. Override them with
`LIGHTSPEED_PLATFORM_ADMIN_EMAIL` and `LIGHTSPEED_PLATFORM_ADMIN_PASSWORD`.
These defaults are local-only and must never be used in a deployed environment.

The server accepts the following primary configuration names:

- `LIGHTSPEED_PLATFORM_DATABASE_URL`;
- `LIGHTSPEED_PLATFORM_AUTH_SECRET`;
- `LIGHTSPEED_PLATFORM_BASE_URL`;
- `LIGHTSPEED_PLATFORM_TRUSTED_ORIGINS`;
- `LIGHTSPEED_PLATFORM_ADMIN_EMAIL` and
  `LIGHTSPEED_PLATFORM_ADMIN_PASSWORD`;
- `LIGHTSPEED_PLATFORM_GITHUB_CLIENT_ID` and
  `LIGHTSPEED_PLATFORM_GITHUB_CLIENT_SECRET`;
- `LIGHTSPEED_PLATFORM_CONFIGURATOR_MCP_URL`; and
- `LIGHTSPEED_PLATFORM_CHANNELS_HEALTH_URLS`.

Imported pre-release aliases were removed as part of the greenfield
product-identity reset. Platform deployments must use the
`LIGHTSPEED_PLATFORM_*` names above.

Live database or Temporal integration tests require explicit opt-in variables
and are not part of the ordinary unit-test run. Never use production connector
credentials for local Telegram or WhatsApp workers.

CI runs those integration boundaries explicitly. To reproduce them:

```bash
LIGHTSPEED_PLATFORM_MIGRATION_TEST_URL=postgres://... npm run test:migrations
npm run test:integration:channels
```

Release construction stages one platform runtime and one Platform workers
runtime. The Platform workers image includes Channels, Bots, and every
connector dependency. It starts a granular role, the `channels` or `bots`
composite, or `all`; connectors remain opt-in through
`LIGHTSPEED_CHANNELS_CONNECTORS`. The release manifest records one digest for
each image.
