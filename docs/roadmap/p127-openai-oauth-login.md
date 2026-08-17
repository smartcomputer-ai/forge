# P127: Subscription Credentials For Coding Agents In Environments (OpenAI + Anthropic)

**Status**

- Proposed 2026-08-17, revised the same day after checking `openai/codex`
  `main` and the Claude Code authentication docs. Not started.
- Builds on [P69](archive/p69-generic-auth-token-broker.md) (auth
  substrate: `auth_flows`, grants, encrypted secrets, refresh, `modelApiKey`
  /`modelOAuth` rows), [P90](archive/p90-multi-tenancy.md),
  P118/P125 (environment jobs, `environments/credentials/bind`,
  `secret_env` injection), and the Integrations page shipped as G0 of
  [the GitHub App roadmap](later/pNNN-platform-github-app-installations.md).
- Closes the "Provider OAuth login" checkbox in [roadmap.md](roadmap.md).
- OpenAI's login endpoints and Codex's auth model are not a public
  contract; re-verify before each slice.

## Goal

A universe owner with a Pro/Max/Plus/Team subscription connects it once in
the Integrations page. The credential lands **encrypted in Lightspeed's auth
tables** (secret + grant with account/plan metadata) and is **injected into
environments** through the existing credential bindings so that **Claude
Code** and **Codex** run there on the subscription, without any login step
inside the environment.

Secondary outcomes from the same place:

- an OpenAI **Platform API key** for Lightspeed's own sessions, obtained by
  sign-in instead of paste (cheap add-on to the OpenAI device flow);
- (optional, flagged, tackled when we get there) Lightspeed's own agent
  talking to the ChatGPT Codex backend with the subscription credential.

## Facts (verified 2026-08-17)

Anthropic / Claude Code:

- Anthropic's OAuth client is exclusive to Claude Code/Claude.ai; no
  third-party registration; consumer OAuth tokens are blocked outside those
  products (server-side since Jan 2026). We must not run their flow.
- **`claude setup-token`** (run by the user on their own machine) performs
  the browser/paste-code login and prints a **one-year OAuth token** meant
  for "CI pipelines, scripts, or other environments where interactive
  browser login isn't available". Set as **`CLAUDE_CODE_OAUTH_TOKEN`**;
  requires Pro/Max/Team/Enterprise; model requests only; not read in
  `--bare` mode. Precedence: below `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`
  /`apiKeyHelper` — do not inject those alongside it.

OpenAI / Codex:

- Public PKCE client `app_EMoamEEZ73f0CkXaXp7hrann`, issuer
  `https://auth.openai.com`; browser redirect pinned to
  `http://localhost:1455/auth/callback` (unusable hosted).
- **Device flow** (usable hosted, "enter a code" UX): `POST
  /api/accounts/deviceauth/usercode {client_id}` → `{device_auth_id,
  user_code, interval, verification_url}`; user opens the URL and enters the
  code; poll `POST /api/accounts/deviceauth/token {device_auth_id,
  user_code}` → `{authorization_code, code_verifier, code_challenge}`;
  exchange at `/oauth/token` with `redirect_uri =
  https://auth.openai.com/deviceauth/callback`.
- **API key exchange:** `/oauth/token` with
  `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`,
  `requested_token=openai-api-key`, `subject_token=<id_token>` → a Platform
  API key.
- **Refresh:** `/oauth/token` JSON `{client_id, grant_type:"refresh_token",
  refresh_token}` → `{id_token?, access_token, refresh_token?}` (rotation
  possible; unmeasured).
- **How Codex takes credentials:**
  - `OPENAI_API_KEY` / `CODEX_API_KEY` env — API key mode.
  - **`CODEX_ACCESS_TOKEN`** env (`codex login --with-access-token`) —
    ChatGPT **Enterprise** workspace access tokens "for trusted scripts,
    schedulers, and private CI runners". Not available on Plus/Pro.
  - **`$CODEX_HOME/auth.json`** — the only carrier for Plus/Pro ChatGPT
    tokens: `{ auth_mode?, tokens: { id_token, access_token,
    refresh_token, account_id }, last_refresh }`. Codex refreshes and
    rewrites it. OpenAI's docs list `scp ~/.codex/auth.json` to a remote
    machine as the supported fallback ("treat like a password").
  - `codex login --device-auth` inside a machine — works but is a login per
    environment; rejected for provisioned environments.
- ChatGPT account id / plan from id_token claim `https://api.openai.com/auth`.
- **Codex → ChatGPT backend request contract** (local checkout
  `ff770113ca`, 2026-08-17; `codex-rs/model-provider-info/src/lib.rs`,
  `model-provider/src/bearer_auth_provider.rs`, `core/src/client.rs`,
  `login/src/auth/default_client.rs`):
  - Base URL `https://chatgpt.com/backend-api/codex`, chosen whenever the
    auth mode is ChatGPT-ish (`Chatgpt | ChatgptAuthTokens | Headers |
    AgentIdentity | PersonalAccessToken`); API-key mode uses
    `https://api.openai.com/v1`. Endpoints `/responses`,
    `/responses/compact`, optional WebSocket v2.
  - Auth headers: `Authorization: Bearer <access_token>` and
    `ChatGPT-Account-ID: <account_id>` (+ `X-OpenAI-Fedramp: true` for
    FedRAMP accounts).
  - Client identity headers: `originator` (default `codex_cli_rs`, sent by
    the shared client; thread override only when different), Codex-shaped
    `User-Agent` (`codex_cli_rs/<version> (<os> <ver>; <arch>) …`),
    provider header `version: <cargo version>`. Per-request advisory
    headers: `session-id`, `thread-id`, `x-codex-installation-id`,
    `x-codex-beta-features`, `x-codex-turn-state`, `x-codex-routing-hint`.
  - Body: `store: false`, `stream: true`, `tool_choice: "auto"`,
    `instructions`, `prompt_cache_key` (session id), `service_tier`,
    `client_metadata` (session/thread metadata). Codex-eligible models only.
  - **Attestation:** `supports_attestation()` is true whenever the cached
    auth is ChatGPT auth. The Rust core does not compute it; it asks the
    *host application* (desktop app / IDE extension that declared
    `capabilities.requestAttestation`) via `attestation/generate` just
    before each backend call (100 ms budget) and forwards
    `x-oai-attestation: {"v":1,"s":<status>,"t":"v1.<opaque>"}`. CLI and
    `codex exec` never send it ("attestation generation is not supported in
    exec mode"). Today the header is optional (status codes cover timeout /
    failure); the shape — per-request opaque token from OpenAI-signed hosts,
    presumably platform device/app attestation — is a collect-and-observe
    rollout that can later gate or rate-limit unattested ChatGPT-auth
    traffic. A third-party client cannot produce it.

Lightspeed today:

- Generic flows, grants, encrypted secrets, refresh with rotation and
  single-flight, `NeedsReauth`; no device-code/manual-code completion, no
  `id_token` storage, no per-provider presets.
- `environments/credentials/bind { envName, source: authGrant |
  authProviderCredential | directSecret }` injects `secret_env` into jobs;
  no file injection (and none is added by this plan).
- Integrations page exists (GitHub card).

## Non-Goals

- Running any provider CLI login inside environments.
- Implementing Anthropic OAuth ourselves.
- File materialization in `lightspeed-envd`. Files, where a tool needs one,
  are written by a one-line bootstrap in the environment from an injected
  env var.
- Reimplementing Codex's backend client (attestation/telemetry). The
  Lightspeed-agent-on-Codex-backend path is optional and flagged (D6).

## Design

### D1. One credential model for both vendors: a `coding_agent_subscription` grant

Store subscription credentials as auth grants (encrypted secrets +
metadata), never as loose environment secrets:

```text
provider_kind      anthropic_claude_code | openai_chatgpt
provider_id        anthropic | openai
external_authorization_id
                   Anthropic: none (opaque token) → grant per paste
                   OpenAI:    chatgpt_account_id (from id_token)
access_token_secret_id
                   Anthropic: the setup-token (one-year)
                   OpenAI:    ChatGPT access token
refresh_token_secret_id
                   OpenAI only
extra secret       OpenAI id_token (kind auth.openai.id_token)
expires_at_ms      Anthropic: paste time + 1y (best effort)
                   OpenAI: access-token expiry
metadata_json      email, plan/tier, workspace/enterprise flag,
                   last_refresh_ms, source (deviceFlow | pasted)
```

The Integrations page shows account/plan/health from this row; Disconnect
revokes it and evicts cached tokens. `NeedsReauth` renders as *Reconnect*.

### D2. Anthropic: paste `claude setup-token`, inject `CLAUDE_CODE_OAUTH_TOKEN`

Card copy: "Run `claude setup-token` on your machine (Pro/Max/Team), paste
the token." Stored as D1. Environment binding
`{ envName: "CLAUDE_CODE_OAUTH_TOKEN", source: authGrant }`. No refresh, no
flow, no file. Guardrail: refuse to bind `ANTHROPIC_API_KEY` /
`ANTHROPIC_AUTH_TOKEN` and `CLAUDE_CODE_OAUTH_TOKEN` into the same
environment (precedence would silently pick the key). Anthropic API keys
for Lightspeed sessions remain the existing `model:anthropic` paste path,
shown on the same card.

### D3. OpenAI: device flow in core, tokens land in the table

New flow kind `openai_device` on `auth_flows` (compiled preset: client id
with env override, endpoints, scopes, extra params; not a user
`auth_clients` row). API:

```text
auth/flows/start { kind: "openaiDevice", outcomes: ["chatgpt" | "apiKey"]+ }
  -> { flowId, userCode, verificationUrl, expiresAtMs }
auth/flows/read  { flowId } -> { status, grantId?, providerId?, error? }
```

Core polls OpenAI from a bounded background task tied to the flow row (TTL
15 min), exchanges the code, then applies outcomes:

- `chatgpt` → D1 grant (`openai_chatgpt`), refresh via a `GrantTokenSource`
  reusing `OAuthRefreshRuntime` locking/rotation order; `invalid_grant` →
  `NeedsReauth`.
- `apiKey` → id_token exchange → write/replace `model:openai`
  `modelApiKey` row (needs `auth/providers/update` with credential replace,
  also wanted for GitHub App key rotation). The token set is not retained
  unless `chatgpt` was also requested.

Fallbacks on the same card: paste the contents of a local `~/.codex/auth.json`
(→ D1 grant, `source: pasted`), or an Enterprise access token (→ D1 grant
with `metadata.enterpriseAccessToken = true`, no refresh).

`auth/flows/complete { flowId, code }` is added as a generic manual-code
primitive at the same time (future CLI browser flow, other vendors).

### D4. OpenAI: injection into environments

- Enterprise access token → binding `{ envName: "CODEX_ACCESS_TOKEN" }`.
- Plus/Pro token set → binding with a **rendered value**: a new
  `EnvironmentCredentialSource::AuthGrantRendered { grant_id, format:
  codexAuthJson }` resolves the grant (refreshing first) and produces the
  auth.json document as the env value, e.g. `CODEX_AUTH_JSON`. The
  environment writes it: one line in the image entrypoint or the profile /
  job pre-command:

  ```sh
  install -d -m 700 "${CODEX_HOME:-$HOME/.codex}" && \
  printf '%s' "$CODEX_AUTH_JSON" > "${CODEX_HOME:-$HOME/.codex}/auth.json" && \
  chmod 600 "${CODEX_HOME:-$HOME/.codex}/auth.json" && unset CODEX_AUTH_JSON
  ```

  Lightspeed ships this snippet as a documented bootstrap and as a default
  in first-party environment templates; the daemon is untouched.

Refresh ownership: Lightspeed refreshes on injection so jobs start fresh;
Codex may refresh inside the environment during long jobs. Whether that
invalidates Lightspeed's copy depends on rotation, which O3 measures live
before choosing between "nothing needed", "report-back message from the
environment", or "accept reconnect". Do not guess.

### D5. Profiles

A profile's environment section can declare default credential bindings so
provisioned environments (P125) get `CLAUDE_CODE_OAUTH_TOKEN` /
`CODEX_ACCESS_TOKEN` / `CODEX_AUTH_JSON` from the universe's connected
subscriptions without per-environment setup. Existing per-environment
`environments/credentials/bind` stays for ad-hoc cases.

### D6. Optional: Lightspeed agent on the ChatGPT Codex backend

Kept as an **optional, flagged experiment** to be tackled when we get there;
never a dependency of anything else in this plan. Concretely:

- Endpoint profile (`base_url` + header overlay) on `ResolvedProviderAuth`;
  the OpenAI Responses client applies it per request (clients stay
  deployment singletons).
- A `model:openai` row bound (`modelOAuth`) to an `openai_chatgpt` grant
  resolves to `https://chatgpt.com/backend-api/codex`, `Authorization:
  Bearer <access>`, `ChatGPT-Account-ID`, `originator`, a Codex-shaped
  `User-Agent`, `session-id`/`thread-id`; body forced to `store: false`,
  `stream: true`, `prompt_cache_key = session id`, `client_metadata`.
- Admission rejects unsupported session config (stored responses,
  `previous_response_id`, non-Codex models) with a typed error; model
  discovery lists Codex-eligible models only when the flag is on.
- `LIGHTSPEED_OPENAI_CHATGPT_BACKEND=on|off`, default `off`; failures never
  fall back silently to an API key.

Why optional: OpenAI's terms tie ChatGPT plans to OpenAI's own surfaces
("API keys are still the recommended default for automation"); the request
contract is undocumented and drifts (headers and body fields above changed
between two reads in one day); and attestation is now wired to ChatGPT auth
and only OpenAI-signed hosts can produce it — once enforced, this path
stops working with no engineering workaround. Codex-in-environments (D4) is
unaffected: `codex exec` itself sends no attestation, so headless CLI use
either stays exempt or OpenAI provides the sanctioned alternative
(Enterprise access tokens already are one). The durable path for our own
loop is the API-key outcome of D3.

## Security Invariants

1. Platform never sees tokens: pastes go browser → Platform → core in one
   request and are encrypted on receipt; device-flow tokens never leave
   core; reads return ids/status/metadata only.
2. Device `user_code` is bound to the starting universe and flow TTL.
3. Subscription grants inject only through explicit bindings; they never
   resolve as model-provider credentials for `api.openai.com` /
   `api.anthropic.com`.
4. Env values are delivered via the existing `secret_env` path (never
   argv, never logged); the auth.json bootstrap unsets the variable after
   writing.
5. Refresh is single-flight per grant; `invalid_grant` → `NeedsReauth`,
   cached tokens evicted; Disconnect revokes and evicts.
6. Universe deletion cascades grants, secrets, and cached tokens.

## Persistence Delta

- `AuthProviderKind`/`auth_grants.provider_kind`: `anthropic_claude_code`,
  `openai_chatgpt` (+ `openai_oauth` for the flow row).
- New secret kind `auth.openai.id_token`.
- `auth_flows`: allow the new kind; device ids in metadata.
- `EnvironmentCredentialSource::AuthGrantRendered { grant_id, format }`
  (additive enum variant + column on the binding table).
- Optional profile field for default bindings.
- No new tables. `external_authorization_id` + uniqueness shared with the
  GitHub App roadmap migration.

## Slices

### S1: Anthropic subscription card (no flow)

- `anthropic_claude_code` grant kind; paste endpoint; Integrations card
  (paste, status, disconnect); `CLAUDE_CODE_OAUTH_TOKEN` binding helper and
  the API-key/OAuth-token conflict guard; profile default binding.
- Live test: environment job runs `claude -p` on the token.

### S2: OpenAI paste paths + Codex injection

- `openai_chatgpt` grant kind (pasted auth.json or Enterprise access token),
  `AuthGrantRendered{codexAuthJson}`, `CODEX_ACCESS_TOKEN` /
  `CODEX_AUTH_JSON` bindings, documented bootstrap snippet + template
  default.
- Live test: environment job runs `codex exec` on a Plus/Pro token set with
  no `OPENAI_API_KEY`.

### S3: OpenAI device flow + refresh + API-key outcome

- `openai_device` flow kind, background poll, `auth/flows/complete`
  primitive, refresh source with rotation, API-key exchange →
  `model:openai` (+ `auth/providers/update`), card gets *Sign in with
  OpenAI*. Live measurement of refresh-token rotation → decide D4
  follow-up.

### S4: CLI parity

- `lightspeed auth anthropic token set`, `lightspeed auth openai login
  [--chatgpt] [--api-key]`, `status`, `logout`.

### S5 (optional, flagged): Lightspeed agent on the Codex backend (D6) —
only when we get there; expected to have a limited shelf life.

## Acceptance Criteria

- Anthropic: paste once; a provisioned environment runs Claude Code on the
  subscription with no login; Disconnect stops the next job from receiving
  the token.
- OpenAI: after sign-in (or paste), a provisioned environment runs Codex on
  the subscription with no login; the auth.json bootstrap leaves a 0600
  file and no token in logs/job output.
- The OpenAI sign-in can additionally mint a Platform API key that
  sessions use immediately.
- Refresh keeps a connected OpenAI account usable for at least the observed
  refresh-token lifetime; revocation surfaces `NeedsReauth` within one
  attempt.
- No token transits Platform in any response (route tests).
- Anthropic API key and OAuth token are never both injected into one
  environment.

## Open Questions

- Do OpenAI refresh tokens rotate (single-use)? Access-token lifetime?
  (S3 live test; decides whether a report-back path is needed.)
- Which OpenAI account types allow the API-key exchange (Team/Enterprise
  may block it)?
- Where does the auth.json bootstrap live by default: environment template
  entrypoint, profile pre-command, or both?
- Should `claude setup-token` expiry (1y) be tracked for a renewal warning?

## References

- `openai/codex` at `ff770113ca` (2026-08-17): `codex-rs/login/src/{server.rs,
  device_code_auth.rs, auth/manager.rs, auth/personal_access_token.rs,
  auth/storage.rs, auth/default_client.rs}`,
  `codex-rs/model-provider-info/src/lib.rs`,
  `codex-rs/model-provider/src/{provider.rs, bearer_auth_provider.rs}`,
  `codex-rs/core/src/{client.rs, attestation.rs}`,
  `codex-rs/app-server/{README.md, src/attestation.rs}`; and
  https://learn.chatgpt.com/docs/auth.
- https://code.claude.com/docs/en/authentication (`claude setup-token`,
  `CLAUDE_CODE_OAUTH_TOKEN`, precedence).
- [P69 G7](archive/p69-generic-auth-token-broker.md), [GitHub App
  roadmap](later/pNNN-platform-github-app-installations.md).
