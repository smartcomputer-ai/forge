# Lightspeed platform

The first-party Lightspeed management plane, web UI, connector host, and
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
- `connectors/` — the connector host: one process serving every enabled
  Telegram and WhatsApp account across universes, discovered through the core
  API, with grant-leased provider tokens and one Temporal activity worker per
  account queue. It replaces the former `bots/`, `channels/`, and `workers/`
  packages; Bots and Channels core now live in the Rust runtime.
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

Specific tool choices in session and profile configuration use registry IDs
(for example, `env.run_process`). The runtime chooses builtin names and schemas
for each model. Transcripts preserve each call's original `toolName` alongside
its optional admitted `toolId`; the UI does not resolve historical names again.
Demo tool fixtures record these identities explicitly as well.

Context views and run outputs share a content descriptor. Assistant messages,
reasoning, and audio transcripts can reference JSON; API views include their
full projected text. Detailed run reads include `output` and `outputText` even
after the message leaves active context. The transcript renders messages and
reasoning directly. Tool previews stay bounded and expand their original bytes
through `blobs/read`.

Conversation input items accept an optional `origin` string (1–200 bytes, not
blank). The message and steering routes derive `user:<id>` from the authenticated
platform session; request bodies cannot override it. Bot deliveries, their media,
and batch framing use `event`, including steering and context-only delivery.
Other API clients may supply other origin strings. Origin is display metadata,
not an authorization claim, and absent origin means unknown. It is persisted on
each input/context entry and returned by context, event, and detailed run reads.
The web transcript styling is unchanged; this field enables later styling without
inferring authorship from the model role or message text.

Session transcripts open with one recent event window from
`session/events/read` with `direction: "backward"`, then follow `session/events/read` strictly after
that initial window's head. Scrolling near the top automatically requests
older windows through an independent exclusive `before` cursor. There is no
history cutoff or load-more button. The message scroller preserves the visible
message when older entries are prepended; its loading sentinel sits outside
the content element so it cannot hide prepends from the scroller.

Windows may split a run or tool batch. Results without a loaded call start
render as continued tool activity and acquire their original metadata as older
pages arrive. Partial generation totals are not presented as whole-run usage.
History is reconstructed chronologically and deduplicated by event/entry ID;
historical lifecycle transitions never overwrite live controls. History errors
retry independently of live polling, and changing sessions aborts both paths.
Live polling retries a transient connection failure immediately with `waitMs: 0`
and at most one event from the unchanged cursor. Only a failed recovery probe shows a disconnect;
an empty successful probe clears it immediately and resumes normal long-polling.
Authorization and event-integrity errors remain visible immediately. Live reads
have a deadline ten seconds beyond their requested wait so stalled connections
cannot stop updates indefinitely.

The authoritative configuration reference is
[`docs/variables.md`](../docs/variables.md), with separate sections for the
Platform server, connector host, Configurator MCP, and development-only
settings.

Platform admins manage invite-only user accounts under **Admin → Users**. They
can update a user's name, verified sign-in email, platform role, and password;
password resets revoke that user's active sessions. Signed-in users can update
their own display name and password under **Account**. Self-service email
changes stay disabled until the deployment provides an email-verification
sender.

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
published as a static site (serve `dist-demo/` under `/demo/` with an
`index.html` fallback for client routes).

```bash
./dev.sh demo         # Vite dev server on http://localhost:5175/demo/ (alias: npm run demo)
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
- `LIGHTSPEED_PLATFORM_CONFIGURATOR_MCP_URL` and the optional
  `LIGHTSPEED_PLATFORM_CONFIGURATOR_MCP_ALLOW_PRIVATE_NETWORK`; and
- `LIGHTSPEED_PLATFORM_CHANNELS_HEALTH_URLS`.

Imported pre-release aliases were removed as part of the greenfield
product-identity reset. Platform deployments must use the
`LIGHTSPEED_PLATFORM_*` names above.

Live database tests require explicit opt-in variables and are not part of the
ordinary unit-test run. Never point a local connector host at production
channel accounts.

CI runs the migration boundary explicitly. To reproduce it:

```bash
LIGHTSPEED_PLATFORM_MIGRATION_TEST_URL=postgres://... npm run test:migrations
```

Release construction stages one platform runtime and one connector-host
runtime (still published as the `platform-workers` image so image references
and manifest keys stay stable). The connector host serves the providers named
by `LIGHTSPEED_CONNECTOR_PROVIDERS` for every account the core reports; see
`connectors/README.md`. The release manifest records one digest for each
image.
