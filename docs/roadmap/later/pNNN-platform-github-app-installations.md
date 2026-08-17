# Deployment-Owned App Identities And Universe Authorizations (GitHub App First)

**Status**

- Later / product integration.
- Written 2026-08-16 after reviewing the universe-scoped GitHub App provider
  and installation-grant implementation; revised 2026-08-17 after verifying
  the design against `crates/auth`, `crates/store-pg`, the gateway, and
  `platform/`.
- Builds on P69's auth substrate and GitHub App token broker, P90's
  multi-tenant boundary and deployment-level state lookup, P118's
  deployment-provider/universe-binding pattern, and P124's first-party
  Platform management plane.
- Not urgent. The universe-owned (BYO) GitHub App path already works. Build
  this when hosted onboarding friction ("Connect GitHub, pick an org, done")
  is an actual product problem. The generic pieces below are worth having
  regardless because deployment-owned OAuth clients for Notion, Linear, Slack,
  and Google Workspace need the same axis.

## Goal

Let one Lightspeed-operated application identity (first: a GitHub App) be
connected to arbitrary external accounts by many universes, without copying
the deployment's private key or client secret into every universe and
without weakening universe isolation.

The design is intentionally generic. GitHub is the first driver, not the
shape of the schema:

```text
Deployment-owned app identity + credentials    deployment configuration (env)
  GitHub App id, private key, OAuth client id/secret, webhook secret
  (later: Notion/Linear/Slack client id/secret, Google client or service key)

Universe-owned external authorization           auth_grants row
  GitHub installation, Notion workspace, Slack team, Google consent

One-time connect flow with public callback      auth_flows + core /auth/callback

Short-lived tokens                              minted or refreshed on demand,
                                                cached in process memory only
```

Keep the existing universe-owned GitHub App path. It remains the route for
bring-your-own Apps, self-hosting, GitHub Enterprise Server, customers that
require an isolated App identity, and the one case the deployment App cannot
serve (below).

### Terminology (read this first)

- **App** — the GitHub-side identity: app id + private key. Owned by the
  deployment. Installed many times.
- **Installation** — GitHub's record that the App is installed on **one**
  GitHub account (a user or an organization), with a repository selection.
  Each has its own numeric `installation_id`. Installing the same App on
  `acme-org` and on `lukas` yields two installations.
- **Installation token** — minted from App key + installation id; reaches
  only that installation's repositories.

Ownership rules that follow:

- One deployment App serves installations in many universes.
- One universe may own many installations (personal account plus several
  organizations). Repositories owned by unrelated accounts require one
  installation per owning account; that is normal universe configuration.
- **One installation belongs to at most one universe.** This is a
  Lightspeed choice (see below), enforced structurally.
- GitHub allows an App to be installed on a given account **only once**.
  Therefore, with the deployment App, one GitHub organization can be
  connected to **one universe**. A customer who needs the same organization
  in two universes uses the BYO path for one of them. Document this in the
  product; it is the same limitation Vercel, Sentry, and Linear live with.

Exclusivity is deliberately chosen over shared installations because it is
simpler (webhook routing resolves to exactly one universe) and because
exclusive → shared later is a one-line index drop, whereas shared → exclusive
later breaks existing claims.

## Current State (verified 2026-08-17)

P69 models both halves inside one universe:

- `auth_providers` stores a GitHub App's non-secret configuration
  (`config_json = { appId, apiBaseUrl }`) and `credential_secret_id`, with an
  `ON DELETE RESTRICT` FK into `auth_secrets`;
- `auth_secrets` stores the AEAD-encrypted private key;
- an `auth_grants` row of kind `github_app` represents one installation; its
  installation id, account login, permissions, and repository selection live
  only in `metadata_json` (`GitHubInstallationGrantMetadata`);
- `GitHubAppRuntime` reads provider + grant, signs the App JWT, exchanges it
  for an installation token, and caches it in a process-memory
  `BTreeMap<AuthGrantId, Token>`;
- `auth_flows` is a generic one-time flow table (hashed `state`, expiry,
  `consumed_at_ms`, `redirect_uri`, `grant_id` result) with provider kinds
  `mcp_oauth | github_app_user | github_oauth_app | custom_oauth`;
- the core gateway hosts the only public ingress in the system,
  `/auth/callback`, and resolves the callback's universe with the
  deployment-level `find_auth_flow_universe(state_hash)` query;
- `auth/github/installations/list` enumerates every installation the
  provider's App JWT can see and `auth/github/installations/grant` creates a
  grant for one of them.

All auth tables (`auth_secrets`, `auth_grants`, `auth_clients`, `auth_flows`,
`auth_providers`) are keyed by `universe_id`. There is no scope concept:
`AuthProviderStore`, `SecretStore`, and `AuthGrantStore` are universe-bound
and take no universe parameter. The only deployment-scoped auth table is
inbound `api_keys` (P90).

Gaps this design must close:

- No column or index carries the installation id; nothing prevents two grants
  for the same installation, even within one universe
  (`github_installation_grant_draft` mints a random grant id with no lookup).
- Installation tokens are minted with **no request body**: no
  `repositories`/`repository_ids`/`permissions` narrowing exists, and the
  cache is keyed by grant id only.
- `AuthGrantStatus` is `Active | NeedsReauth | Revoked | Failed`; there is no
  `Suspended`, and `InstallationNotFound` (uninstall) is currently mapped to
  `NeedsReauth`, which is a mislabel.
- Platform has no public/unauthenticated route, no webhook receiver, and no
  GitHub App code beyond passthrough to `auth/providers/*` and better-auth
  social login.

## Design Decisions

### D1. Deployment app identity comes from environment configuration

The deployment provider is the same record as a universe `auth_providers`
row, constructed once per process from `LIGHTSPEED_*` variables and held in
deployment-shared runtime state (alongside P90's `DeploymentStores` /
`DeploymentClients`). Secrets are env-managed too, exactly like
`LIGHTSPEED_AUTH_SECRETS_MASTER_KEY` and the environment-daemon bearer token:

```text
LIGHTSPEED_GITHUB_APP_PROVIDER_ID          logical id referenced by grants
LIGHTSPEED_GITHUB_APP_ID
LIGHTSPEED_GITHUB_APP_API_BASE_URL
LIGHTSPEED_GITHUB_APP_PRIVATE_KEY_FILE     (or inline / secret-manager ref)
LIGHTSPEED_GITHUB_APP_CLIENT_ID            install / user-OAuth callback
LIGHTSPEED_GITHUB_APP_CLIENT_SECRET_FILE
LIGHTSPEED_GITHUB_APP_WEBHOOK_SECRET_FILE
```

If the variables are unset, the platform-App feature is off. Rotation is
change-config-and-redeploy.

Code reads it through a small trait (`DeploymentAuthProviderSource` or
similar) so that a Postgres-backed implementation with `operator/auth/
providers/*` can replace the env source later without touching grant, flow,
callback, or webhook code. **No `deployment_auth_providers`,
`deployment_auth_clients`, or deployment secret table now.** They become
worthwhile only for several apps per deployment, operator UI registration
without redeploy, or online rotation with status tracking.

Do not create a "system universe" to hold the deployment provider. Do not
replicate its private key or client secret into universe `auth_secrets`.

### D2. The external authorization stays an auth grant, with two new columns

Do not add `github_installations`, `github_install_intents`,
`github_installation_repositories`, or per-vendor inventory tables. The
installation is the external authorization already represented by
`auth_grants`. Promote the identity out of opaque metadata:

```text
auth_grants
  provider_scope             text NOT NULL DEFAULT 'universe'
                             ('universe' | 'deployment')
  external_authorization_id  text        (GitHub installation id; later Notion
                                          workspace id, Slack team id, ...)
```

Constraints (both partial, both generic):

```sql
-- no duplicate grants for one authorization within a universe (BYO too)
UNIQUE (universe_id, provider_id, external_authorization_id)
  WHERE external_authorization_id IS NOT NULL;

-- one deployment-app authorization is owned by at most one universe;
-- doubles as the webhook reverse-lookup index
UNIQUE (provider_id, external_authorization_id)
  WHERE provider_scope = 'deployment'
    AND external_authorization_id IS NOT NULL;
```

`provider_id` already exists on `auth_grants`. Provider scope must be
explicit; never infer deployment ownership from a provider-id prefix.
`external_authorization_id` is generic auth-grant vocabulary from the start
(open question resolved: other deployment-owned integrations need it).

For a platform GitHub App grant:

```text
universe_id                    owning universe
provider_scope                 deployment
provider_id                    LIGHTSPEED_GITHUB_APP_PROVIDER_ID
provider_kind                  github_app
external_authorization_id      GitHub installation id
metadata_json                  account id/login/type, permissions,
                               repository selection, non-secret observations
```

Do not scan or filter installation ids out of `metadata_json`.

### D3. Grant status gains `suspended`; uninstall becomes `revoked`

Add `Suspended` to `AuthGrantStatus` and the SQL check. GitHub suspension is
an installation lifecycle condition, not an OAuth refresh failure. In the
same slice, change the existing `InstallationNotFound → NeedsReauth` mapping
to `Revoked` (reinstalling produces a new installation id and therefore a new
grant).

### D4. The connect intent is an `auth_flows` row, hosted by core

There is no Platform intent table. `auth_flows` already is a one-time,
hashed-state, expiring, universe-scoped intent with a deployment-level
reverse lookup and a public callback. Extend it with one column so a flow can
reference the deployment OAuth client (there is no `auth_clients` row for the
platform App):

```text
auth_flows
  client_scope   text NOT NULL DEFAULT 'universe'   ('universe' | 'deployment')
```

Enable "Request user authorization (OAuth) during installation" on the
deployment App. GitHub then delivers `code`, `installation_id`,
`setup_action`, and `state` to a single callback URL, so the setup callback
*is* the OAuth callback and the user-token proof the flow needs falls out of
the existing `github_app_user` flow kind.

### D5. Core is authoritative; Platform is a membership gate plus UI

Core (Rust runtime) owns:

- the deployment provider identity and credentials (from env);
- the universe-owned authorization grant and its status;
- exclusive `(provider, external_authorization_id)` ownership;
- the connect flow, its state, and the public callback;
- live verification of the authorization before creating or reactivating a
  grant;
- token minting, narrowing, in-memory caching, and eviction;
- the public webhook receiver, signature verification, delivery
  deduplication, and reverse lookup from external authorization id to
  universe.

Placing callback and webhooks in core is not optional: both need deployment
secrets (client secret, webhook secret) that must not be copied into
Platform, and core already hosts the only public ingress and the reverse
lookup. This also keeps CLI/headless deployments and Platform-down operation
working.

Platform (TypeScript) owns:

- authenticating the human;
- checking universe owner/admin membership (better-auth `member.role`);
- starting the flow through the ordinary universe-scoped API and redirecting
  the browser;
- rendering authorizations, status, and repository views from core
  projections.

Platform keeps no authoritative mirror of authorization ownership,
permissions, or webhook state, and gets no new tables. Platform → core is the
existing trusted-header path; no new deployment credential is introduced.

### D6. Existing enumeration/grant methods are BYO-only

With a shared App, `auth/github/installations/list` under the App JWT would
enumerate every customer's installations, and `auth/github/installations/
grant` would let any universe member claim one without a flow. Both must
reject `provider_scope = deployment` providers. Deployment-App claims happen
only inside the callback (D7).

### D7. Vendor-specific behavior lives behind a driver trait

Vendors differ in signature scheme, where the external id appears, how to
prove authority, and event → lifecycle mapping. Keep that behind a small
per-`provider_kind` trait, not in schema or API shape:

```text
ExternalAuthorizationDriver
  authorization_id_from_callback(query, token_response) -> external id
      GitHub: installation_id query param
      Notion / Slack: workspace_id / team.id in the token response
  verify_authorization_live(deployment provider, external id, user token)
      GitHub: external id ∈ GET /user/installations, then App-JWT lookup
      Notion / Linear: no-op
  verify_inbound_event(headers, body) -> (external id, delivery id, event)
      GitHub: X-Hub-Signature-256 HMAC, X-GitHub-Delivery, installation.* events
```

Token acquisition already dispatches per kind through `GrantTokenSource`.
GitHub App remains one implementation; a Google service-account
domain-wide-delegation source would be a second; Notion, Linear, and Slack
reuse the existing OAuth refresh path with a deployment-scoped client and
need no new Rust driver beyond id extraction.

## Persistence Delta

Everything fits existing tables:

- `auth_grants`: `provider_scope`, `external_authorization_id`, two partial
  unique indexes, backfill (`provider_scope = 'universe'`;
  `external_authorization_id` from validated `github_app` metadata, rejecting
  duplicates before the constraint is added; invalid legacy metadata leaves
  the grant unusable and visible to operators, never guessed).
- `auth_grants.status` and `AuthGrantStatus`: add `suspended`.
- `auth_flows`: `client_scope`.
- No new core tables. No Platform tables. Webhook delivery dedup may start as
  a bounded in-memory set keyed by delivery id plus idempotent reconcile; a
  generic `inbound_deliveries` ledger is optional later.

## Connect Flow

1. A universe owner or admin selects **Connect GitHub** in Platform.
2. Platform checks membership and calls the universe-scoped flow-start method
   with `client_scope = deployment`, kind `github_app_user`, and the return
   URL.
3. Core creates the `auth_flows` row (hashed `state`, expiry) and returns the
   GitHub App installation URL with `state`; Platform redirects.
4. GitHub lets the user choose an account and `all` or selected repositories.
5. GitHub redirects to core `/auth/callback` with `code`, `installation_id`,
   `setup_action`, `state`.
6. Core resolves the universe from `state_hash`, consumes the flow once,
   exchanges `code` for a user token, verifies `installation_id` is among the
   user's accessible installations, reads the installation live with the App
   JWT, and creates the grant under the unique index (idempotent for the same
   universe; rejected for another universe).
7. Core redirects to the flow's return URL; Platform shows the result from
   core projections.
8. Consumers resolve tokens through the normal broker.

The callback's `installation_id` is untrusted input; GitHub documents that
it can be spoofed. Steps 6's user-token check plus live App lookup are the
proof. A signed webhook may assist reconciliation but never binds an
installation to a universe without a matching flow.

## Token Minting And Repository Scope

Installation tokens remain short-lived and are never persisted. The broker
branches on `provider_scope`:

```text
universe    -> universe AuthProviderStore + SecretStore (unchanged)
deployment  -> env-loaded deployment provider with matching provider_id
```

Add request-body narrowing to `GitHubApiClient::create_installation_token`
(`repository_ids`, `permissions`) and make the cache key
`(grant_id, narrowing fingerprint)`. Narrowing may only preserve or reduce
what GitHub granted the installation; caller-supplied repository names, ids,
or permission maps never widen it. Evict all of a universe's cached tokens on
universe purge (P90) and on grant revoke/suspend.

## Webhook Lifecycle

Core `/webhooks/github` verifies the signature with the deployment webhook
secret before parsing, deduplicates by delivery id, resolves the universe via
the deployment-scope unique index (exactly one or fail closed), and
reconciles the grant:

- installation created: observe; never claim without a matching flow;
- permissions accepted/changed: refresh permission metadata;
- suspended: `Suspended`, block token resolution, keep ownership;
- unsuspended: revalidate live, restore `Active`;
- deleted: `Revoked`, evict cached tokens;
- repositories added/removed: refresh selection metadata, invalidate affected
  narrowed-token cache entries.

Repository events route by installation id, never by repository name. Events
for unknown or unclaimed installations may be logged but create no universe
state.

## API Shape

Universe-scoped (universe inferred from request context per P90; also
exposed by Configurator MCP as read/disconnect only):

```text
auth/flows/start                     (existing; gains client_scope)
auth/authorizations/list|read        grants with external ids, status,
                                     account/permission observations
auth/authorizations/disconnect       revoke + evict
auth/github/repositories/list        per-authorization accessible repos
                                     (GitHub-specific because narrowing is)
```

Existing `auth/github/installations/list|grant` stay for BYO providers only
(D6). Trusted operator surface (never in Configurator MCP):

```text
operator/auth/authorizations/lookup      external id -> universe
operator/auth/authorizations/reconcile
operator/auth/authorizations/revoke
```

No `claim` operator method (claims happen in the callback) and no
`operator/auth/providers/*` until the deployment provider moves out of env.
Do not reuse `auth/providers/create` for the platform App; that method
remains the BYO path with a universe-owned key.

## Security Invariants

1. One deployment-app authorization is owned by at most one universe.
2. A universe may hold many authorizations.
3. Deployment app credentials (private key, client secret, webhook secret)
   live only in deployment configuration; never in universe storage or in
   Platform.
4. Tokens are never persisted and never returned by management APIs.
5. A callback parameter alone never proves ownership; a consumed one-time
   flow plus live verification does.
6. Only a universe owner/admin may start a connect flow.
7. Webhook signatures and delivery ids are verified before routing.
8. Unknown, ambiguous, suspended, revoked, or cross-universe authorizations
   fail closed before provider I/O.
9. Numeric account/repository ids are identity; logins and names are display
   metadata.
10. Narrowing only reduces granted access.
11. Deleting a universe cascades its grants and evicts its cached tokens; it
    never touches the deployment app identity.
12. Deployment-scope providers are never enumerable or claimable through the
    BYO installation methods.

## Implementation Slices

### G1: Deployment provider from env + provider scope

- Env-loaded deployment GitHub App provider behind a source trait.
- `provider_scope` on `auth_grants`; broker branch; BYO path untouched.
- Reject deployment-scope providers in `auth/github/installations/*`.
- Tests: same provider id in both scopes; missing env → feature off; fail-
  closed lookup.

### G2: External authorization identity

- `external_authorization_id` + backfill + both partial unique indexes.
- `Suspended` status; uninstall → `Revoked`.
- Deployment-level lookup by `(provider_id, external_authorization_id)`.
- Tests: concurrent cross-universe claims yield one owner; in-universe
  reconnect is idempotent.

### G3: Connect flow in core

- `client_scope` on `auth_flows`; flow start with the deployment client.
- Callback handles `installation_id`/`setup_action`, user-token check, live
  lookup, claim, redirect.
- Platform: membership gate, start-and-redirect, result page.
- Tests: spoofed/replayed callback, wrong-universe state, expired flow.

### G4: Webhooks

- Core `/webhooks/github`; signature, dedup, reverse lookup, reconcile,
  cache eviction.

### G5: Consumption

- `auth/authorizations/*`, repository listing, narrowed minting with the
  compound cache key, Platform views for connected/suspended/needs-upgrade.

### G6 (when needed): Second vendor

- Deployment-scoped OAuth client (Notion or Linear) through the same
  `client_scope`/`provider_scope` axis, proving no GitHub-specific schema
  leaked.

## Acceptance Criteria

- One deployment App serves installations in two universes with no
  universe-stored key.
- Each universe lists and uses only its own authorizations.
- Concurrent claims of one installation by two universes yield exactly one
  owner; the loser fails closed.
- A spoofed or replayed callback cannot create or move a binding.
- An installation suspended or deleted at GitHub is unusable before the next
  repository operation and cannot return a cached token.
- Repository-selection changes are reflected without recreating the grant.
- Deleting universe A affects neither the deployment App nor universe B.
- BYO GitHub Apps continue to mint through the universe-scoped path.
- Tokens resolve while Platform is unavailable.
- `npm run check:identity` passes (no new deployment inputs outside
  Lightspeed-owned names).

## Resolved Questions

- One deployment App, configured via env, secrets via env/secret-manager
  reference; tables only if operator-managed multi-app arrives.
- `external_authorization_id` is generic vocabulary.
- Suspension is a grant status.
- Core hosts callback and webhook ingress.
- Installations are exclusive per universe; the one-org-one-universe
  limitation is documented and BYO is the escape hatch; relax later by
  dropping one index if product needs it.

## Open Questions

- Webhook delivery retention: in-memory dedup only, or a generic ledger?
- When repository inventory becomes first class, which domain owns it (auth,
  a source-repository catalog, or workspace import)?
- Which vendor is second, and does it need `verify_authorization_live` at all?

## References

- [P69 generic auth and token broker](../archive/p69-generic-auth-token-broker.md)
- [P90 multi-tenancy](../archive/p90-multi-tenancy.md)
- [P118 environment domain and lifecycle](../p118-environment-domain-and-lifecycle.md)
- [P124 first-party Platform monorepo](../p124-first-party-platform-monorepo.md)
- [GitHub: sharing a GitHub App](https://docs.github.com/en/apps/sharing-github-apps/sharing-your-github-app)
- [GitHub: setup URL security](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
- [GitHub: installation access tokens](https://docs.github.com/en/rest/apps/apps#create-an-installation-access-token-for-an-app)
- [GitHub: webhook events and payloads](https://docs.github.com/en/webhooks/webhook-events-and-payloads)
