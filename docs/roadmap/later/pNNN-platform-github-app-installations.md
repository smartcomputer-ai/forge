# Platform GitHub App And Universe Installations

**Status**

- Later / product integration.
- Written 2026-08-16 after reviewing the existing universe-scoped GitHub App
  provider and installation-grant implementation.
- Builds on P69's GitHub App token broker, P90's multi-tenant boundary, P118's
  deployment-provider/universe-binding pattern, and P124's first-party
  Platform management plane.

## Goal

Support one Lightspeed-operated GitHub App that customers can install on
arbitrary GitHub user or organization accounts and connect to one universe,
without copying the App private key into every universe or weakening
universe isolation.

The load-bearing scope split is:

```text
GitHub App identity and private key       deployment-scoped
GitHub App installation                  universe-scoped
Installation grant and repository access universe-scoped
Installation token                       short-lived, minted on demand
```

A universe may own multiple installations, for example one personal-account
installation and installations for several organizations. One GitHub
installation must belong to at most one universe.

Keep the existing universe-owned GitHub App path. It remains useful for
bring-your-own Apps, self-hosting, GitHub Enterprise Server, and customers
that require an isolated App identity.

## Current State

P69 models both parts inside one universe:

- `auth_providers` stores a GitHub App's non-secret configuration and a
  reference to its private-key secret;
- `auth_secrets` stores the encrypted private key;
- an `auth_grants` row of kind `github_app` represents one installation and
  carries its installation id, account, permissions, and repository selection
  in non-secret metadata; and
- `GitHubAppRuntime` reads that provider and grant, signs an App JWT, exchanges
  it for an installation token, and caches the token only in process memory.

All three auth tables are keyed by `universe_id`. This is correct for a
universe-owned developer App, but a hosted product App has one deployment
identity shared by installations in many universes. Duplicating its private
key into every universe would create unnecessary secret copies, complicate
rotation, and confuse provider ownership.

The runtime already has deployment-scoped concepts. Inbound API keys are
deployment-scoped because they resolve a caller to a universe, and environment
compute uses a deployment provider plus universe-scoped bindings. A
platform-operated GitHub App should follow the same shape.

## Design Decisions

### The installation remains an auth grant

Do not add a second durable `github_installations` catalog for the first
slice. A GitHub App installation is the external authorization represented by
the existing universe auth grant. Splitting the same lifecycle between a
GitHub installation row and an auth grant would create two sources of truth
for ownership, status, permissions, and revocation.

Promote the installation id out of opaque metadata into an indexed external
authorization identity. Candidate additions to `auth_grants` are:

```text
provider_scope                 universe | deployment
provider_id                    logical provider id
external_authorization_id      provider-native authorization id
```

For a platform GitHub App grant:

```text
universe_id                    owning universe
provider_scope                 deployment
provider_id                    platform GitHub App provider
provider_kind                  github_app
external_authorization_id      GitHub installation id
metadata_json                  account id/login/type, permissions,
                               repository selection, non-secret observations
```

Enforce exclusive ownership structurally with the equivalent of:

```sql
UNIQUE (provider_id, external_authorization_id)
WHERE provider_scope = 'deployment'
  AND provider_kind = 'github_app';
```

This index also supports the cross-universe reverse lookup required by
webhooks. Do not scan or filter installation ids out of `metadata_json`.

Provider scope must be explicit. Do not infer deployment ownership from a
reserved provider-id prefix.

### Add a deployment provider source

The runtime needs a provider source above the universe boundary. A candidate
durable representation is:

```text
deployment_auth_providers
  provider_id
  provider_kind
  display_name
  config_json
  credential_ref
  status
  created_at_ms
  updated_at_ms
```

The first implementation may instead load the single platform App identity
from deployment configuration and the deployment secret manager. The table is
needed when Lightspeed must support multiple deployment Apps, operator API
management, status, or online credential rotation. In either form, the token
broker resolves a grant according to `provider_scope`:

```text
universe   -> existing universe AuthProviderStore and SecretStore
deployment -> deployment provider source and deployment credential source
```

Do not create a fake "system universe" to hold the deployment provider. Do
not replicate its private key into universe `auth_secrets` rows.

Current `auth_secrets` rows require a universe. For the initial hosted product,
the deployment provider may reference an external deployment secret. Add a
deployment-scoped encrypted-secret table only if recoverable credential CRUD
and rotation must be owned by Lightspeed itself; do not add it merely to make
the schema resemble the universe secret store.

### Core is authoritative

The provider and installation authorization belong in Lightspeed core even
though the product setup experience lives in Platform.

Core is authoritative for:

- deployment GitHub App provider identity, status, and private-key reference;
- the universe-owned installation grant;
- exclusive `(provider, installation)` ownership;
- live installation validation and reconciliation;
- installation status and non-secret permission/account observations;
- installation-token minting, narrowing, and in-memory caching; and
- reverse lookup from a GitHub installation id to its universe for trusted
  webhook routing.

This keeps the broker's authorization decision and credential resolution in
one domain, permits CLI and headless deployments, and prevents agent execution
from depending on the Platform database or TypeScript server.

Platform owns the human-facing control flow:

- authenticating the user;
- checking universe owner/admin membership;
- creating a short-lived, one-time installation intent;
- redirecting the user to GitHub;
- handling the setup callback;
- verifying that the returning user may associate the installation;
- receiving and authenticating GitHub webhooks when Platform hosts the public
  webhook endpoint; and
- invoking trusted core operator methods to claim, reconcile, suspend, or
  revoke an installation grant.

Platform must not keep an authoritative mirror of installation ownership or
permissions. It may retain webhook delivery/deduplication records and setup
workflow state. Reads shown in the product should come from core projections.

There are no cross-database foreign keys. Platform passes stable universe,
provider, installation, intent, and delivery identifiers through idempotent
operator calls.

## Minimal Persistence Shape

The intended first slice does not require a family of GitHub-specific core
tables.

### Core runtime database

1. Add a deployment auth-provider table if the provider is managed durably;
   otherwise use deployment configuration plus a secret-manager reference.
2. Extend `auth_grants` with explicit provider scope and indexed external
   authorization identity.
3. Add the partial uniqueness constraint that prevents an installation from
   being claimed by two universes.

### Platform database

Add a short-lived installation-intent table, or an equivalent durable
one-time-state mechanism:

```text
github_install_intents
  intent_id
  state_hash
  universe_id
  initiated_by_user_id
  expires_at
  consumed_at
  created_at
```

Store only a hash of the externally transmitted `state` value. Intents expire,
are one-time-use, and cannot be completed for another universe.

A webhook-delivery ledger keyed by GitHub's delivery id is optional but useful
for replay protection, diagnostics, and retry-safe processing. Raw webhook
payload retention is not required for installation authority.

### Deferred tables

Do not add `github_installation_repositories` initially. Fetch or reconcile
the accessible repository list from GitHub and keep repository selection and
summary observations in grant metadata.

Add a normalized repository table only when repository inventory becomes a
first-class Lightspeed resource that needs search, policy, stable selection,
or event-driven synchronization. At that point repository ids, not mutable
`owner/name` strings, are the external identity.

## Installation Flow

1. A universe owner or admin selects **Connect GitHub** in Platform.
2. Platform creates a one-time installation intent bound to the universe and
   initiating user.
3. Platform redirects to the public GitHub App installation URL with the
   opaque `state` value.
4. GitHub lets the user choose a user or organization account and `all` or
   selected repositories.
5. GitHub redirects to the configured setup URL with an installation id.
6. Platform consumes the matching intent and verifies the initiating user's
   authority over both the Lightspeed universe and the GitHub installation.
7. Platform calls a trusted core operator method to claim the installation for
   the universe.
8. Core authenticates as the deployment App, reads the installation live from
   GitHub, rejects an existing claim by another universe, and creates or
   idempotently returns the universe auth grant.
9. Subsequent consumers resolve the grant through the normal token broker.

The setup callback's `installation_id` is untrusted input. GitHub explicitly
warns that callers can spoof it. The flow must validate the installation using
an authenticated GitHub relationship, such as a GitHub App user token proving
that the initiating user can access the installation, followed by a live App
lookup. A signed webhook may assist reconciliation but must not silently bind
an installation to a universe without the matching Platform intent and
membership check.

One installation belongs to one GitHub user or organization account. Access
to repositories owned by unrelated accounts requires multiple installations;
that is a normal universe configuration.

## Token Minting And Repository Scope

Installation tokens remain short-lived and are never persisted durably. The
broker signs an App JWT using the deployment credential and exchanges it for a
token for the universe grant's installation id.

When an installation is exclusively bound to one universe, the installation's
GitHub-side repository selection is the primary boundary. Where a consumer
needs only one repository, request a token narrowed to that repository id and
to the minimum required permissions. Narrowing is defense in depth and becomes
mandatory if a future design ever subdivides one installation between
multiple internal principals or resources.

Never allow caller-supplied repository names, ids, or permission maps to widen
what GitHub granted to the installation. Token-mint requests may only preserve
or reduce installation access.

Token cache keys must include enough identity to prevent reuse across grants
or narrowed scopes. A cache keyed only by installation grant is insufficient
once different repository or permission subsets can be minted concurrently.

## Webhook Lifecycle

The public webhook receiver verifies the GitHub signature before parsing or
routing an event. Processing is idempotent by GitHub delivery id.

Installation lifecycle events reconcile the core grant:

- installation created: observe it, but do not claim it without a matching
  setup intent;
- permissions accepted or changed: refresh the grant's permission metadata;
- suspended: prevent token resolution while preserving the ownership record;
- unsuspended: revalidate live and restore token resolution;
- deleted: revoke or tombstone the grant and evict cached tokens; and
- repositories added or removed: refresh repository-selection observations
  and invalidate any affected narrowed-token cache entries.

Repository events route by the authenticated App identity and installation id,
not by repository name. The reverse lookup must return exactly one universe or
fail closed. Events for unknown or unclaimed installations may be logged and
reconciled, but must not create universe state implicitly.

The current generic grant states may need a distinct `suspended` state. Do not
mislabel GitHub suspension as `needs_reauth`: it is an installation lifecycle
condition, not a user OAuth refresh failure.

## API Shape

The exact method names remain open, but the boundary should distinguish
universe consumption from deployment administration.

Platform-facing universe operations:

```text
github/installations/list
github/installations/read
github/installations/repositories/list
github/installations/disconnect
```

Trusted operator operations:

```text
operator/github/providers/*
operator/github/installations/claim
operator/github/installations/lookup
operator/github/installations/reconcile
operator/github/installations/revoke
```

Ordinary universe APIs infer the universe from authenticated request context,
as required by P90. They never accept an arbitrary universe id. Cross-universe
lookup and mutation are trusted operator surfaces and must not be exposed by
Configurator MCP.

Do not reuse the current universe `auth/providers/create` method to register
the platform App. That method remains the BYO path and its private key remains
universe-owned.

## Security Invariants

1. One deployment GitHub installation is owned by at most one universe.
2. A universe may have multiple installations.
3. The platform App private key is never copied into universe storage.
4. Installation tokens are never persisted and never returned by management
   APIs.
5. A setup callback parameter alone never proves installation ownership.
6. Only a universe owner/admin may start or complete an installation binding.
7. Core verifies the installation live before creating or reactivating a
   grant.
8. Webhook signatures and delivery identities are verified before routing.
9. Unknown, ambiguous, suspended, deleted, or cross-universe installations
   fail closed before provider or repository I/O.
10. GitHub account and repository numeric ids are authoritative external
    identities; logins and names are display metadata.
11. Token narrowing can only reduce GitHub-granted repositories and
    permissions.
12. Removing a universe deletes or revokes its installation grants and cached
    tokens but never deletes or rotates the deployment App identity.

## Migration And Compatibility

Existing universe-owned GitHub App providers and grants continue to resolve
with `provider_scope = universe`. Backfill existing grants accordingly.

Platform-managed installations use `provider_scope = deployment`. Do not
automatically convert BYO providers or deduplicate private keys across
universes: their ownership and rotation semantics are intentionally different.

If `external_authorization_id` is introduced as a generic string, backfill it
from validated `github_app` grant metadata and reject duplicate installation
claims before adding the uniqueness constraint. Invalid legacy metadata should
leave the grant unusable and visible to operators for repair rather than
guessing an installation identity.

## Implementation Slices

### G1: Deployment provider resolution

- Add the deployment provider/credential source.
- Add explicit provider scope to grants and the broker.
- Preserve the existing universe provider path.
- Cover same provider ids in different scopes and fail-closed lookup.

### G2: Structured installation ownership

- Add and backfill `external_authorization_id`.
- Add the global partial uniqueness constraint.
- Add deployment-level lookup by provider plus installation id.
- Add cross-universe isolation and concurrent-claim tests.

### G3: Platform installation flow

- Add one-time installation intents and membership checks.
- Add the GitHub redirect and setup callback.
- Verify the returning user's access to the installation.
- Claim the installation through an idempotent operator API.

### G4: Webhook reconciliation

- Verify webhook signatures and deduplicate deliveries.
- Reconcile installation deletion, suspension, permission changes, and
  repository-selection changes.
- Evict affected token-cache entries.
- Route repository events by authenticated installation identity.

### G5: Product consumption

- Add universe installation and repository views.
- Let repository-aware tools select only repositories visible through the
  universe's installations.
- Mint narrowly scoped tokens where the consumer can state its repository and
  permission requirements.
- Surface disconnected, suspended, and permission-upgrade states clearly.

## Acceptance Criteria

- One deployment App credential serves installations in at least two
  universes without copying the private key into either universe.
- Each universe lists and uses only its own installations and repositories.
- Concurrent attempts to claim the same installation for different universes
  result in exactly one owner.
- A spoofed setup callback cannot create or move an installation binding.
- An installation deleted or suspended at GitHub becomes unusable before the
  next repository operation and cannot return a cached token.
- Repository-selection changes are reflected without recreating the provider
  or installation grant.
- Removing universe A does not affect the deployment App or universe B's
  installations.
- Existing universe-owned/BYO GitHub Apps continue to mint tokens through the
  original universe-scoped credential path.
- The runtime can resolve installation tokens while Platform is unavailable.

## Open Questions

- Is one deployment App sufficient for the first hosted product, allowing its
  provider configuration to remain deployment config, or is operator-managed
  multi-App support required immediately?
- Should deployment credentials use an external secret-manager reference or a
  new deployment-scoped encrypted-secret store?
- Should `external_authorization_id` be generic auth-grant vocabulary or a
  typed GitHub-only column? Prefer generic vocabulary only if another concrete
  provider needs the same identity.
- Should installation suspension extend `AuthGrantStatus`, or should external
  provider lifecycle be modeled separately from generic token health?
- Which component hosts the public webhook endpoint? Platform is the natural
  product ingress, but core remains authoritative regardless of ingress
  placement.
- How much webhook delivery history is operationally useful, and what is its
  retention policy?
- When repository inventory becomes first class, does it belong to the auth
  domain, a future source-repository catalog, or a workspace-import domain?

## References

- [P69 generic auth and token broker](../p69-generic-auth-token-broker.md)
- [P90 multi-tenancy](../p90-multi-tenancy.md)
- [P118 environment domain and lifecycle](../p118-environment-domain-and-lifecycle.md)
- [P124 first-party Platform monorepo](../p124-first-party-platform-monorepo.md)
- [GitHub: sharing a GitHub App](https://docs.github.com/en/apps/sharing-github-apps/sharing-your-github-app)
- [GitHub: setup URL security](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
- [GitHub: installation access tokens](https://docs.github.com/en/rest/apps/apps#create-an-installation-access-token-for-an-app)
- [GitHub: webhook events and payloads](https://docs.github.com/en/webhooks/webhook-events-and-payloads)
