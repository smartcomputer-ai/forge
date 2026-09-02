# P148 — Key-Based Outbound Environment Registration

**Status**

- Proposed 2026-09-01.
- Builds on P119's environment protocol, gateway route, incarnation fencing,
  and canonical `lightspeed-envd` runtime. It adds one new environment source
  whose daemon opens the data-plane connection outbound.
- Replaces the direct-enrollment sketch in P117 for this scope: the only
  outbound registration credential is a reusable, universe-scoped
  registration key. There are no one-time enrollment tickets or pre-created
  pending environments.
- Harbor is an initial motivating use case, not part of this item.
  [P149](p149-harbor-end-to-end-agent-evaluation.md) defines the Harbor agent
  adapter, trial lifecycle, benchmark configuration, and result handling.

## Goal

Let an operator start arbitrary machines, VMs, containers, or pods with a
publicly reachable Lightspeed gateway URL and one registration key:

```text
LIGHTSPEED_ENVD_GATEWAY_URL=wss://lightspeed.example/environment-gateway/connect
LIGHTSPEED_ENVD_REGISTRATION_KEY=<secret>
```

`lightspeed-envd` connects outbound, authenticates with the key, and becomes a
new environment in the key's universe. The same key may register one daemon or
many unrelated daemons concurrently. Key policy may bound total registrations,
active environments, lifetime, identity modes, and ephemeral cleanup without
changing the image or bootstrap command.

The operator does not manage a client certificate or pre-create one
environment per machine. `envd` generates its own daemon key pair, Lightspeed
assigns the logical environment and incarnation ids, and the gateway returns a
registration receipt. Persisting the daemon key reconnects the same logical
environment; losing it deliberately creates a new one.

This supports:

- a VM or physical host that remains the same environment across restarts;
- a Kubernetes StatefulSet whose `envd` identity lives on stable storage;
- replaceable pods or batch workers that become new environments per instance;
- externally provisioned compute pools whose members all receive the same
  registration key; and
- Harbor started on a developer machine while its task sandboxes run in local
  Docker or any supported remote compute backend and connect to hosted
  Lightspeed.

## Problem

P119 implements `envd` as a passive WebSocket listener. An external environment
stores a reachable daemon endpoint, and Lightspeed dials it on demand. That is
simple on a private network but is a poor fit for Docker Desktop, NATed VMs,
Kubernetes pods, and hosted sandbox providers:

- every environment needs an inbound address or provider-specific tunnel;
- port publication and reachability differ by compute backend;
- a public passive daemon currently relies on deployment network controls
  rather than application authentication; and
- a general pool cannot be bootstrapped from one image and one shared secret.

A Harbor-specific bridge could translate Lightspeed environment calls into
Harbor's environment interface, but that would not exercise the real `envd`
process/PTY/filesystem/job implementation and would create a second execution
adapter to maintain. Outbound registration makes the canonical daemon usable
directly instead.

## Decision

Add **registered environments** alongside the existing provisioned and passive
external environment sources:

```text
environment source
├── provisioned       provider lifecycle + provider-mediated envd route
├── external          stored endpoint; Lightspeed dials passive envd
└── registered        envd connects outbound with a registration key
```

The registered path has these fixed rules:

1. A registration key belongs to exactly one universe and authorizes creation
   of registered environments in that universe. A daemon never chooses or
   claims a universe.
2. A registration key is reusable unless its stored policy limits it. One key
   may admit many distinct daemon identities and environments.
3. `envd` generates a local asymmetric daemon key pair. Lightspeed derives the
   `daemonId` from the public key and proves possession during every initial
   registration or reconnect.
4. Lightspeed, not `envd`, allocates `environmentId` and `incarnationId`. A
   shared registration key holder cannot name an existing environment or take
   it over.
5. A first-seen daemon identity creates one environment atomically. A known,
   active daemon identity reconnects that same environment and does not consume
   another registration.
6. The registration key admits a new daemon identity; it is not the daemon's
   permanent data-plane credential. After registration, the daemon private key
   authenticates reconnects without requiring an operator-managed certificate.
7. Persistent and ephemeral identities have different disconnect semantics,
   but use the same wire protocol and registration keys.
8. Client-supplied names and metadata are correlation aids only. They never
   select a universe, environment id, incarnation, authority, or routing
   destination.
9. Registered daemons keep one outbound data-plane connection while available.
   Filesystem, process, PTY, and job calls remain direct gateway traffic and
   never become Temporal activities or workflows per call.

This reverses P119's “no daemon-initiated transport” decision only for the new
registered source. Existing passive external and provider-mediated routes stay
valid and unchanged.

## Identity Model

Five identities describe separate authorities and lifetimes:

| Identity | Assigned by | Lifetime and purpose |
|---|---|---|
| `registrationKeyId` | Lightspeed | Identifies one reusable universe-scoped admission policy; never sent as authority without its secret |
| `daemonId` | Lightspeed from the `envd` public-key fingerprint | Identifies one daemon installation or ephemeral instance |
| `environmentId` | Lightspeed | Stable universe-owned logical environment bound to one daemon identity |
| `incarnationId` | Lightspeed | Fences the current admitted generation of that environment |
| `connectionId` | Gateway | Identifies one live outbound socket and changes on reconnect |

The registration secret is not an identity. Multiple daemons intentionally
present the same secret. The daemon public key distinguishes them.

### Why the server assigns the environment id

`environmentId` is a universe-owned resource id used by profiles, sessions,
credentials, close operations, and gateway routes. Allowing an untrusted
daemon to choose it would create collision and takeover semantics: a holder of
one shared key could try to register as an existing environment. The gateway
therefore accepts only a daemon public key, identity mode, and bounded
descriptive metadata. The server allocates the logical ids and returns them in
the receipt.

The durable create request id is derived server-side from the daemon identity,
so retries of the same first registration converge on the same environment.
The exact id string is an implementation detail; it must not depend on a
client-supplied hostname or correlation value.

### One daemon identity, one environment

A daemon public key may be bound to at most one non-closed registered
environment in the deployment. Presenting another registration key cannot move
that identity to another universe or environment. Moving a machine deliberately
requires closing/revoking the old environment and resetting the local daemon
identity before registering again.

Two simultaneous sockets proving the same daemon identity represent reconnect
races or a copied private key, not two environments. The gateway admits one
current connection, assigns a new `connectionId`, and fences the superseded
socket. Base images and snapshots must never contain an already generated
daemon private key.

## Persistent and Ephemeral Identities

Identity mode is immutable for one registered environment and is included in
the authenticated registration request. The registration-key policy supplies a
default and an allowed set; `envd` may request an allowed override.

### Persistent

Persistent mode is for a machine identity expected to survive daemon and host
restarts:

- `envd` atomically creates its private key in its configured state directory
  before first registration and refuses persistent mode if it cannot read and
  write that state with restrictive permissions; the operator is responsible
  for placing the directory on storage that survives the intended restarts;
- a reconnect proves the same daemon key and receives the same
  `environmentId` and `incarnationId`;
- disconnect marks the environment unavailable/offline but never closes it
  merely because time passed; and
- closing the environment revokes the daemon binding. The same local identity
  cannot silently reopen it; registering as a new environment requires an
  explicit identity reset.

A Kubernetes StatefulSet is the common container example, but the StatefulSet
name or ordinal is not the cryptographic identity. Stable behavior requires the
`envd` state directory to be mounted on storage that follows the StatefulSet
member, normally a PVC created by `volumeClaimTemplates`. A replacement Pod
that mounts the same state reconnects the same Lightspeed environment.

### Ephemeral

Ephemeral mode is for replaceable pods, batch workers, benchmark sandboxes, and
other instances that should disappear from Lightspeed after they disappear
from the compute backend:

- `envd` generates a fresh daemon key for the instance. It may retain the key
  in instance-local state so an `envd` restart or brief network interruption
  can reconnect during that instance's lifetime;
- the first connection creates a new server-assigned environment;
- disconnect marks it unavailable and begins the key policy's bounded
  ephemeral disconnect grace period;
- reconnect with the same daemon key before expiry cancels cleanup and resumes
  the same environment; and
- expiry closes the environment and revokes the daemon binding. A replacement
  instance with a new daemon key creates a new environment.

A regular Kubernetes Deployment or Job naturally gets this behavior when the
identity is held only in memory or on Pod-local storage such as `emptyDir`.
`emptyDir` can preserve identity across a container restart inside the same
Pod, but a replacement Pod receives new storage and therefore registers a new
environment.

The default identity mode is part of registration-key policy rather than a
global daemon default. A key intended for a disposable pool can therefore
bootstrap pods with only the gateway URL and registration key, while a VM or
StatefulSet key can default to persistent mode. A key may allow both modes when
the caller explicitly configures each daemon.

## Registration Keys

### Durable record

Add a universe-scoped registration-key record with at least:

```text
registration_key_id
universe_id
display_name
key_prefix
secret_hash
status                  active | revoked
default_identity_mode   persistent | ephemeral
allowed_identity_modes
max_registrations       optional; counts newly bound daemon identities
max_active_environments optional; counts non-closed descendants
ephemeral_disconnect_grace_ms
expires_at_ms           optional
registration_count
created_at_ms
updated_at_ms
last_used_at_ms          optional
revoked_at_ms            optional
```

The raw secret is generated with cryptographic randomness, returned once, and
never stored. Reuse the existing API-key secret hashing and display-prefix
conventions unless implementation review finds a reason to strengthen both
together. Registration count and active-capacity admission are checked and
updated atomically with creation of the environment and daemon binding.

Known-daemon reconnects do not increment `registration_count`, do not consume
`max_registrations`, and remain allowed after non-cascading registration-key
revocation. They authenticate with the daemon key, not with continued authority
from the registration key.

### Management API

Add trusted universe-management methods:

```text
environments/registration-keys/create
environments/registration-keys/read
environments/registration-keys/list
environments/registration-keys/revoke
```

Create returns the raw secret once and uses a redacted `Debug` implementation.
Read and list return the display prefix, policy, counters, status, and audit
times, never the hash or secret. Rotation is create-new then revoke-old; no
method reveals or mutates secret material in place.

Revoke has explicit behavior:

- without cascade, reject new daemon identities while already registered
  daemon identities may reconnect; and
- with cascade, also close every non-closed environment admitted by the key and
  revoke its daemon identity.

These methods must never enter the Configurator MCP or any other model-facing
tool catalog. The Platform and CLI may expose them to authenticated human or
service administration surfaces. API responses, traces, audit records, and
errors must not contain the raw registration secret.

Unlimited reuse is supported by absent limits; it is not a bypass around rate
limits. The gateway applies bounded per-key connection and failed-auth rates in
addition to stored capacity limits.

## Daemon Configuration

Support a minimal image/bootstrap contract:

```text
LIGHTSPEED_ENVD_GATEWAY_URL
LIGHTSPEED_ENVD_REGISTRATION_KEY
LIGHTSPEED_ENVD_IDENTITY_MODE             optional; otherwise key default
LIGHTSPEED_ENVD_REGISTRATION_NAME         optional
LIGHTSPEED_ENVD_REGISTRATION_METADATA     optional bounded JSON object
LIGHTSPEED_ENVD_REGISTRATION_RECEIPT      optional output-file path
```

Also support `LIGHTSPEED_ENVD_REGISTRATION_KEY_FILE` so Kubernetes and VM
bootstrap systems can mount the secret instead of placing it directly in
process environment. Supplying both direct and file forms is an error.

The direct environment-variable form is a required convenience path: a
preconfigured image may need no input other than the public gateway URL and
shared key. `envd` reads the secret once, never writes it into its state
directory or receipt, excludes it from diagnostics, and removes registration
configuration variables from every process and job environment it spawns.

An environment with sufficient OS privilege can inspect or compromise the
daemon process and its local identity. No in-guest secret representation can
protect against root in the same security domain. For less-trusted workloads,
prefer a mounted secret visible only during bootstrap, a distinct service
user/process namespace, or a sidecar/VM boundary; constrain the reusable key's
capacity and lifetime. The automatic daemon-key exchange narrows a post-
bootstrap compromise from “register arbitrary new environments” to “impersonate
this environment” once the registration key is no longer reachable.

## Outbound Registration Handshake

`envd` connects to one public gateway route over WSS. Plain WS is accepted only
for explicit loopback/development configuration. The registration secret never
appears in a URL, query string, close reason, or log field.

The authenticated handshake is separate from the existing environment data-
plane initialization but reuses its version and capability types:

```text
envd                                      environment gateway
  │                                               │
  ├──────────── WebSocket connect ───────────────►│
  │◄──────── bounded nonce/challenge ─────────────┤
  ├─ register {                                   │
  │    protocolVersion,                           │
  │    registrationKey? ,                         │
  │    daemonPublicKey,                           │
  │    signature(challenge),                      │
  │    identityMode?, displayName?, metadata?     │
  │  } ──────────────────────────────────────────►│
  │                                               ├─ authenticate/admit
  │                                               ├─ create or reconnect
  │◄─ accepted { environmentId, incarnationId,    │
  │             daemonId, connectionId, mode } ───┤
  │◄════════ environment-protocol requests ═══════╡
```

For a first-seen daemon public key, `registrationKey` is required and must be
active, unexpired, and within policy. For a known daemon identity, proof of its
private key is sufficient; a supplied registration key cannot change its
universe or binding. The gateway bounds unauthenticated connection count,
handshake duration, frame size, and authentication attempts before allocating
an environment.

Admission of a new daemon is one transaction:

1. resolve and verify the registration-key secret;
2. derive `daemonId` from the proved public key and confirm it is unbound;
3. atomically enforce total and active limits;
4. allocate `environmentId` and `incarnationId`;
5. insert the registered environment, incarnation, and daemon binding;
6. increment key audit counters; and
7. return the receipt.

The registration transaction creates the environment in `Booting`. It becomes
`Ready` only after the ordinary data-plane initialization negotiates the
current protocol, bounded roots, and capabilities. A disconnect before that
point follows the normal identity-mode rule: a persistent environment remains
unavailable and retryable by the same daemon identity, while an ephemeral one
closes after its disconnect grace. A retry cannot create a duplicate because
the durable daemon binding already exists.

After authentication, `envd` serves the same JSON-RPC filesystem, process,
PTY, idle, and durable-job methods it serves on an accepted passive socket.
Refactor the daemon around an authenticated WebSocket stream; do not fork a
second method implementation for client mode.

## Environment Record and Routing

Extend the environment source enum with a connection-free registered variant:

```text
Registered {
    registration_key_id,
    daemon_id,
    identity_mode,
}
```

Registered environments have no provider, provider binding, provider target,
template, or stored daemon endpoint. They have no provider power controls or
provider-managed public ingress. Their availability comes from the gateway's
current authenticated connection observation.

Persist the daemon public key and revocation state in a dedicated daemon-
identity record bound to universe, environment, and incarnation. The
environment source contains stable provenance, while raw socket ownership,
request correlation, backpressure, and `connectionId` remain ephemeral gateway
facts. PostgreSQL may hold a short-lived owner lease for multi-gateway routing,
but such a lease never makes a disconnected environment Ready and is never
authoritative lifecycle state.

Temporal workers continue to connect to the stable internal route:

```text
/environment-gateway/routes/{universe}/{environment}/{incarnation}
```

For a registered source, the gateway proxies those frames over the current
outbound daemon socket. Every routed request is fenced by universe,
environment, incarnation, daemon, and current connection. A route with no
current connection fails as unavailable; it never falls back to a different
environment registered by the same key.

The gateway instance owning an outbound socket must be discoverable by other
gateway instances. The first implementation may run the environment-gateway
connection owner as a singleton, but multi-instance production support
requires authenticated internal forwarding or an equivalent short-lived owner
lease. Do not route individual environment calls through Temporal or store
socket payloads in PostgreSQL.

Unlike passive environments, an available registered environment holds one
long-lived outbound socket while idle. Use WebSocket ping/pong, bounded idle
health checks, reconnect backoff with jitter, frame/output limits, and explicit
backpressure. Network disconnect is availability loss, not environment
closure by itself.

## Correlation Metadata and Registration Receipt

An operator or orchestrator may supply a bounded display-name hint and bounded
string metadata when starting `envd`. Examples include:

```json
{
  "orchestrator": "harbor",
  "harbor.jobId": "job-...",
  "harbor.trialId": "trial-...",
  "kubernetes.namespace": "agents",
  "kubernetes.podUid": "...",
  "deployment.member": "worker-3"
}
```

The daemon may add bounded implementation facts such as hostname, OS,
architecture, and `envd` version. It must not scrape all environment variables,
cloud instance metadata endpoints, or mounted Kubernetes credentials. Client
metadata is size/count bounded, rejects control characters, and cannot use the
reserved `lightspeed.*` namespace.

The gateway records accepted values in the environment's existing metadata
map at first registration. They are descriptive and queryable but never:

- authentication input;
- an idempotency or uniqueness key;
- a substitute for the daemon public key;
- authority to select an existing environment; or
- a routing key.

The accepted response is also written atomically to the configured receipt
path and emitted as one bounded structured log event:

```json
{
  "environmentId": "environment_...",
  "incarnationId": "incarnation_...",
  "daemonId": "daemon_...",
  "connectionId": "connection_...",
  "identityMode": "ephemeral"
}
```

It contains no registration secret or private-key material. Orchestrators use
the receipt to target the exact environment they started even when many
instances register concurrently with the same key. Harbor will use this
mechanism, but its adapter and cleanup contract are specified separately.

## Lifecycle and Failure Semantics

- **New daemon + valid key:** create one environment and return its receipt.
- **Known daemon reconnect:** return the same environment/incarnation with a
  new connection id; do not increment registration counters.
- **Unknown daemon + revoked/expired/exhausted key:** reject before creating
  durable environment state.
- **Known daemon after non-cascading key revocation:** allow reconnect based on
  daemon-key proof.
- **Known daemon after environment close or daemon revocation:** reject. An
  explicit local identity reset plus an active registration key is required to
  create a new environment.
- **Persistent disconnect:** mark unavailable and retain the environment until
  explicit close.
- **Ephemeral disconnect:** mark unavailable, start the configured grace
  period, and close unless the same daemon identity reconnects first.
- **Gateway restart:** `envd` reconnects with exponential backoff and jitter.
  Durable identity binding prevents duplicate environments.
- **Duplicate live daemon identity:** one connection becomes current and the
  old connection is fenced and closed.
- **Key limit race:** one atomic transaction admits only the allowed number;
  losers receive a typed capacity refusal.
- **Metadata collision:** display names need not be unique; metadata cannot
  affect identity. Server ids remain authoritative.

Closing a registered environment closes its current socket through the
gateway, revokes its daemon binding, and disables session selection. It does
not terminate the underlying VM, Pod, or host; those remain owned by the
external orchestrator.

## Security Boundary

A reusable registration key delegates the ability to add execution capacity to
one universe. A leaked unlimited key can create many attacker-controlled
environments that may later receive commands or injected credentials if a
profile or user selects them. Treat key creation and distribution like
cluster-join credentials, not like a harmless correlation value.

Required controls:

- TLS server authentication for every non-loopback gateway URL;
- cryptographically random secrets, stored only as hashes and returned once;
- explicit universe binding, expiry, revocation, capacity, and rate limits;
- atomic limit enforcement under concurrent registrations;
- proof of daemon private-key possession before binding or reconnecting;
- server-assigned environment/incarnation ids and strict current-incarnation
  fencing;
- no secret in URLs, metadata, receipts, traces, errors, command lines, child
  process environments, or persisted daemon state;
- bounded pre-authentication work, frames, metadata, output, and connection
  counts;
- audit of key creation, successful new registrations, refusals, last use,
  revocation, and cascade close without storing the secret;
- explicit cleanup of ephemeral environments and operator-visible stale
  persistent environments; and
- a test proving that an identity baked into two images cannot create two
  environments or serve two current connections.

Registration by itself grants no general Lightspeed API access and reveals no
universe resources beyond the assigned environment receipt and bounded
gateway protocol errors. The registration key is not accepted on the normal
JSON-RPC API, model-provider endpoints, provider controller endpoints, or
worker routes.

## Implementation Slices

### 1. Domain, store, and management API

- Add registration-key records, secret mint/hash/redaction, create/read/list/
  revoke methods, policy validation, audit counters, and atomic capacity
  enforcement.
- Add `Registered` environment source and daemon-identity storage with domain
  validation and memory/PostgreSQL parity.
- Keep registration-key methods out of Configurator MCP and generated
  model-facing catalogs while retaining normal generated Rust/TypeScript
  management clients.
- Regenerate the public API contract and TypeScript consumers.

### 2. Outbound gateway and daemon connector

- Add the public WSS registration route, bounded challenge handshake, daemon
  proof, create-or-reconnect transaction, live connection registry, and stable
  worker-route proxy.
- Refactor `lightspeed-envd` so its existing request handler can run over an
  accepted passive socket or a daemon-initiated authenticated socket.
- Add local key generation/persistence, persistent/ephemeral mode, receipt
  output, heartbeat, reconnect backoff, and registration metadata.
- Scrub registration configuration from spawned process and job environments
  and all diagnostics.

### 3. Lifecycle and administration

- Project gateway connect/disconnect observations into Ready/unavailable
  status without treating observations as durable truth.
- Add persistent offline behavior, ephemeral grace/auto-close, explicit daemon
  revocation, and cascading registration-key revoke.
- Add Platform and CLI creation, listing, rotation/revocation, copy-once secret
  display, mode/limit configuration, and descendant-environment visibility.

### 4. Deployment and live acceptance

- Publish the public gateway route through the hosted ingress with TLS and
  bounded metrics.
- Document VM, plain Docker, Kubernetes Deployment/Job, and StatefulSet + PVC
  bootstrap examples using the same daemon image.
- Prove hosted Lightspeed with externally started compute, restart/reconnect,
  concurrent shared-key registration, capacity refusal, cleanup, and cascade
  revocation.
- Decide and implement the multi-gateway connection-owner routing mechanism
  before running more than one environment-gateway owner in production.

## Tests

### Domain and API

- Create returns one redacted secret; read/list never expose secret/hash;
  revoked, expired, malformed, and capacity-exhausted keys fail with typed
  outcomes.
- One shared key concurrently creates distinct server-assigned environments
  for distinct daemon public keys.
- Registration and active limits are atomic under concurrency; reconnects do
  not consume them.
- Registration-key methods cannot appear in Configurator MCP or session tools.
- Non-cascading and cascading revocation have the documented descendant
  behavior.

### Identity and lifecycle

- Persistent identity reconnects after daemon and gateway restart with the
  same environment/incarnation and a new connection id.
- Ephemeral replacement with a new daemon key creates a new environment;
  disconnect grace closes the old one; reconnect inside the grace cancels
  cleanup.
- A closed/revoked daemon identity cannot resurrect its old environment even
  when the original registration key remains active.
- A copied daemon identity yields one current connection and never two
  environments.
- A registration key cannot move a known daemon identity between universes.

### Protocol and security

- Protocol-version/capability negotiation and the complete existing envd
  filesystem/process/PTY/job conformance suite pass over outbound transport.
- Invalid signatures, oversized metadata/frames, handshake timeout, replayed
  challenges, and authentication floods fail before environment creation.
- Environment, incarnation, daemon, and connection fencing rejects stale
  sockets and cross-environment requests.
- Registration secrets do not appear in `Debug`, logs, close frames, receipts,
  persisted daemon state, or child process/job environments.
- Metadata is bounded, reserved names are rejected, and correlation values
  never influence identity or routing.

### Live

- Multiple local containers using one key register distinct environments and
  execute commands through hosted Lightspeed.
- A disposable Kubernetes-style instance is closed after disconnect grace; a
  StatefulSet-style instance reusing the same daemon state reconnects the same
  environment.
- Gateway restart and transient network loss recover without duplicate
  environments or duplicate side effects.
- Many concurrent registrations respect per-key limits and leave no partial
  environment rows after refusals.

## Non-Goals

- One-time enrollment tickets, short-lived per-environment bootstrap tokens,
  or pre-creating a pending environment before `envd` connects.
- Letting `envd` choose its universe, `environmentId`, `incarnationId`, or an
  existing environment to claim.
- Using client-supplied hostname, Pod UID, VM id, or Harbor identifiers as
  authentication or durable identity.
- Replacing passive external environments or provider-mediated Incus routes.
- Defining the Harbor adapter, benchmark suite, task correlation workflow, or
  result publication; those belong to
  [P149](p149-harbor-end-to-end-agent-evaluation.md).
- Requiring operator-managed X.509 certificates or a deployment PKI. A later
  transport may encode the same daemon-key proof in mTLS without changing the
  registration-key or identity model.
- Kubernetes ServiceAccount, cloud workload-identity, SPIFFE, or provider-
  native attestation. They may become alternate registration authorities
  later; reusable keys are the only authority in this item.
- Provisioning or deleting the underlying VM, container, Pod, or host.
- Provider power control, public application ingress, environment pools,
  scheduling, billing, or capacity placement for registered environments.
- Making the registration key a general universe API credential or a
  permanent data-plane bearer credential.
