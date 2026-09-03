# P152 — envd Release and Distribution

**Status**

- Proposed 2026-09-03, from deploying the `harbor` merge to production and
  running the first Harbor evaluation ([P149](p149-harbor-end-to-end-agent-evaluation.md))
  against it. Reviewed the same day against the code; the review moved
  self-upgrade from an open idea to Slice 4 and corrected the Slice 1 test
  design.
- Slices 1 to 3 implemented 2026-09-03: explicit connector on both trust
  paths with the provider install in the daemon library; the musl target
  through `release/metadata.env`, the build image, `build-dist.sh`, and both
  workflows; `envd.json` written and verified by the manifest scripts;
  `--print-build`; build facts on `ImplementationInfo`, `ServerInfo`, and
  the environment row at every admission; smoke checks for the packaged
  daemon's build facts and its TLS client. Verified by the unit suites, the
  live registration suite, and a musl build in the pinned image that runs
  on `debian:bullseye`, `ubuntu:22.04`, and `alpine:3.20`. The Harbor
  adapter's target map, build script, and exclusion list were updated in
  its own repository. Open in ls.bot: the well-known route with the archives
  beside it, and the protocol-number notice in its manifest validation.
  Slice 4 implemented 2026-09-03: `lightspeed-envd upgrade` resolves the
  deployment document, streams and verifies the target archive, checks the
  candidate's build facts, and atomically replaces the executable; registered
  daemons can opt into the same operation on protocol mismatch and re-exec
  once per process lineage. Focused coverage uses a local discovery/archive
  server and verifies URL policy, installation, protocol pinning, CLI/config,
  and the non-opted-in diagnostic. The cross-repository live protocol-bump
  acceptance waits for the open ls.bot serving work.
- Behavior of the daemon at runtime is [P151](p151-exec-leftover-processes.md);
  this item is about the binary: how it is built, what it links, how it is
  found, how a running deployment identifies itself, and how a long-lived
  daemon follows a gateway across a protocol change.

## Goal

Anyone who needs to place `lightspeed-envd` on a machine, a VM, a container,
or a benchmark sandbox can find the artifact that matches a deployment, verify
it, run it on any Linux image, and prove which build they ran. A daemon that
stays registered for a long time can follow its gateway across a protocol
change without an operator logging in.

## Problems observed

1. **The release binary panics on its first TLS connection.** The release is
   one workspace build; feature unification compiles both rustls providers
   (`ring` through `object_store`, `aws-lc-rs` through envd's own deps), so
   rustls has no process default. envd only consults that default on the
   no-CA path: `TlsSettings::from_config(None)` returns no connector, so
   tokio-tungstenite builds its own `ClientConfig` and panics with `Could not
   automatically determine the process-level CryptoProvider`. The CA-file
   path builds its config with an explicit provider and never hit the bug.
   `reqwest` uses the process default too. A single-crate
   `cargo build -p environment-daemon` only sees `aws-lc-rs` and works,
   which is why local builds, the unit suite, and the live registration suite
   never showed it. Every Harbor trial on the release binary failed before
   registering.
2. **glibc pins the sandbox floor.** The release builds on
   `rust:1.97.1-bookworm` and links glibc 2.36. Terminal-Bench images on
   `debian:bullseye` (glibc 2.31) fail `envd --version`; two of 89 tasks had
   to be excluded. Any customer image older than bookworm has the same
   problem.
3. **Nothing points at "the current build".** The deployed commit is only in
   hz01's `/var/lib/ls-deploy/last-good.env`. A `main` snapshot exists only
   as an OCI bundle on GHCR, pushed with `oras` and pulled through
   `infra/scripts/fetch-lightspeed-binary` with registry tooling and
   credentials; there is no plain HTTPS download for it. Only tagged releases
   have public asset URLs. Production deploys snapshots, so an orchestrator
   elsewhere has no way to ask "which envd matches this gateway" and fetch it.
4. **The server and the handshake do not say what they are.** The binary
   already does: `lightspeed-envd --version` prints version, git sha, target,
   and rustc through `release_info::LONG_VERSION`, and `scripts/release/smoke.sh`
   runs it. But the API's `initialize` returns `serverInfo.version: "0.1.0"`
   with no sha; the registration and data handshakes send
   `implementation.version` from `CARGO_PKG_VERSION`, which is `0.1.0` for
   every build, and no sha; and the gateway writes `lightspeed.envd.version`
   into the environment row only when it creates the environment, never on a
   reconnect, so a replaced daemon keeps showing its first build forever.
5. **A protocol bump strands registered daemons.** Compatibility is one
   integer compared for equality on both sides. When the gateway moves it,
   every registered daemon sees the new number in the challenge, reports a
   terminal `unsupportedProtocol`, and exits; a supervisor restarts it into
   the same rejection. The daemon has no way to learn where the matching
   build is. The first production deploy after P151 (protocol 2) is the
   first such flag day.

## Decision

1. envd never depends on the process-level rustls provider for its gateway
   dials. `TlsSettings` always builds an explicit connector with the
   `aws-lc-rs` provider and the bundled WebPKI roots, adding the operator's
   CA file when given. The `install_default` call stays for `reqwest` and
   moves into the library so tests exercise it. A unit test asserts the
   no-CA path yields a connector; the packaged binary is smoked against a
   TLS listener.
2. The published `envd` is a static binary: `x86_64-unknown-linux-musl` is
   the acceptance target; `aarch64-unknown-linux-musl` is built when the
   pinned image can cross-compile it. The glibc build stops being published
   for envd; server, provider, and CLI keep their targets.
3. Every release and `main` snapshot carries a small discovery document in
   the bundle. Serving it, and the archives it names, is the deployment's
   job, not the gateway's: ls.bot publishes both at a well-known path
   through Caddy, next to the site and demo artifacts it already serves by
   digest. The gateway and the API do not know about the path.
4. Compatibility is decided by the protocol number alone, exact match,
   enforced by both sides; the git sha is provenance and never gates.
   Different envd builds coexist under one gateway as long as they speak the
   current protocol. The server, the handshakes, and the environment row all
   report the protocol number and the build, and the row is updated on every
   admission.
5. A registered daemon can install the build its gateway names, on demand
   through `lightspeed-envd upgrade` and, when opted in, automatically on a
   protocol mismatch. Provider-managed VMs are not upgraded in place; they
   follow ls.bot's rebuild-by-incarnation plan
   (`docs/environment-daemon-upgrade-plan.md` in that repository).

## Design

### Compatibility model

- One integer, `CURRENT_PROTOCOL_VERSION`, is the whole contract. The daemon
  compares it against the gateway's challenge before it signs anything; the
  gateway compares it on the register frame; every data socket compares it
  in both directions on `initialize`; the Incus provider controller handshake
  does the same. All four comparisons are exact.
- The gateway is the authority, because it is the side a daemon's owner
  cannot change. The daemon's own check is an early exit that avoids signing
  for a gateway that will refuse anyway. No version ranges: every accepted
  old number is code the gateway carries forever.
- Additive behavior goes through `EnvironmentCapabilities`, which is how
  native search, glob, and ranged reads landed without a bump. Only a
  wire-incompatible change moves the integer, as P151 did from 1 to 2.
- Identity checks (registration key, known daemon key, environment not
  closed) decide whether this daemon identity may attach, not which binary.
  Version strings and shas appear in metadata and logs only.
- A bump is a release event. `validate-release-manifest` and the deploy
  script print when the bundle's protocol number differs from the installed
  release's, because that is the moment registered daemons stop connecting.

### Provider selection and the TLS test

- `TlsSettings::from_config` builds a `rustls::ClientConfig` through
  `builder_with_provider(aws_lc_rs)` on both paths and always returns a
  connector; the CA-file branch only adds trust anchors to the same config.
  Nothing in the dial path can reach tungstenite's default connector.
- `rustls::crypto::aws_lc_rs::default_provider().install_default()` moves
  from `main` into the library entry the binary and tests share (idempotent,
  result ignored), so `reqwest` has a provider in a workspace build.
- Unit test in `crates/environment-daemon`: `from_config(None)` yields a
  connector. It fails on any change that reintroduces the `None` path.
- `scripts/release/smoke.sh` runs the packaged envd once against a local
  TLS listener (a self-signed acceptor is enough) so the packaged binary,
  not a rebuilt one, is what passes. CI already runs
  `cargo test --workspace`, which is the feature-unified build.

### Static targets

- Add `x86_64-unknown-linux-musl` to `release/metadata.env` as the envd
  target; `build-dist.sh` builds envd for it and packages
  `lightspeed-envd-<version>-<target>.tar.gz` with a checksum. `aws-lc-rs`
  builds on x86_64 musl with the pinned image's cmake and clang.
- `aarch64-unknown-linux-musl` needs a musl cross C toolchain the pinned
  bookworm image does not carry. Add it when that is cheap, using `ring`
  for that target only if `aws-lc-rs` will not cross-build. Harbor sandboxes
  and hz02 VMs are x86_64, so nothing waits on it.
- The Incus image recipe on hz02 and the Harbor adapter consume the musl
  archive; the glibc floor disappears from both documents.

### Discovery

- Each bundle gains `envd.json`:

  ```json
  {
    "version": "0.1.0",
    "gitSha": "2093b949…",
    "channel": "release",
    "protocolVersion": 2,
    "builtAtMs": 1756900000000,
    "artifacts": {
      "x86_64-unknown-linux-musl": {
        "file": "lightspeed-envd-0.1.0-x86_64-unknown-linux-musl.tar.gz",
        "sha256": "…",
        "url": "https://github.com/smartcomputer-ai/lightspeed/releases/download/v0.1.0/lightspeed-envd-0.1.0-x86_64-unknown-linux-musl.tar.gz"
      }
    }
  }
  ```

  `channel` is `release` or `main`. `url` is set when a plain HTTPS download
  exists, which today is only the GitHub release asset of a tag; in a
  snapshot bundle it is `null`. `builtAtMs` lives here and only here, so
  the binaries stay reproducible. Producing the document is the release
  pipeline's part (`scripts/release/create-manifest.mjs` alongside
  `release-manifest.json`); `validate-release-manifest` checks it.
- Serving belongs to the deployment. In ls.bot, `install-deployment`
  copies the installed bundle's `envd.json` and envd archives under the
  versions directory, fills every `url` with the archive it serves itself,
  and Caddy serves the document at `GET /.well-known/lightspeed-envd` and
  the archives beside it, switched atomically with the release like
  `/demo/`. A served document therefore always has absolute URLs; only a
  bundle read directly may carry `null`. That is an ls.bot change, tracked
  there; this item guarantees the document and the archives exist in every
  bundle.
- `lightspeed-envd --print-build` prints its own
  `{name, version, gitSha, target, protocolVersion}` as JSON, the same
  facts as `--version`. It does not hash itself: the manifest hashes the
  archive, and the downloader verifies that.
- An orchestrator's path: fetch the document from the deployment's public
  host, pick its target, download, verify `sha256`, run `--print-build`, and
  check `protocolVersion` against what the gateway's `initialize` reports.
  No credentials, no checkout.

### Self-identification

- `ServerInfo` gains `gitSha` and
  `envd: {version, gitSha, protocolVersion, targets}`; `version` becomes
  `0.1.0+2093b949` style. The envd block describes the daemon this release
  ships, not the only daemon accepted. No download URLs, hashes, or
  timestamps in the API: `initialize` sits behind a bearer key and cannot
  serve unauthenticated discovery, and the binary is built before the URLs
  exist. `targets` comes from `release-info` at build time; a local build
  reports its host target. Nothing in the platform or the TypeScript client
  reads `serverInfo.version`, so the suffix is safe.
- `ImplementationInfo` gains optional `gitSha` and `target`; envd fills
  `version` from `release_info::VERSION` instead of the crate version. The
  type is shared by the registration frame, the data `initialize`, and the
  provider controller handshake, so provider-managed VMs report their build
  too, which the ls.bot rebuild plan wants recorded per incarnation.
- The gateway writes `lightspeed.envd.version`, `lightspeed.envd.gitSha`,
  and `lightspeed.envd.protocolVersion` into the environment row on every
  admission, reconnects included, where it already stamps the new
  connection. The reserved prefix keeps clients from setting them.
- Acceptance is protocol equality plus "the envd sha equals the one the
  gateway's own document names", not "envd sha equals server sha": an
  envd-only hotfix must not fail the check. Provenance records both shas.

### Self-upgrade

Three daemon populations, three answers:

- **Ephemeral sandboxes** (Harbor): the orchestrator places a fresh binary
  per trial from the discovery document. Self-upgrade is meaningless.
- **Provider-managed VMs** (hz02): inbound on the private bridge, no
  registration identity, and the unit's `ProtectSystem=strict` makes the
  binary's directory read-only to the service. ls.bot's plan replaces the
  VM through a new incarnation from a new image; an in-place updater is at
  most an attended emergency tool there. This item adds nothing for them.
- **Registered long-lived daemons** (a dev box, a customer machine, any VM
  registered by key): the only population where self-upgrade pays, and the
  one problem 5 strands.

For the third population:

- `lightspeed-envd upgrade`: derive `https://<gateway host>/.well-known/lightspeed-envd`
  from the configured gateway URL (`LIGHTSPEED_ENVD_DISCOVERY_URL` overrides
  it for deployments that serve the document elsewhere), pick the entry for
  the running binary's target, download, verify `sha256`, run the new
  file's `--print-build` and check its `protocolVersion` and `gitSha`
  against the document, then replace the executable atomically: write the
  new file next to the running one and rename over it. If that directory
  is not writable, refuse and print the manual command. The operation
  installs whatever the gateway names, so a downgrade is the same command.
- Opt-in automatic mode, `LIGHTSPEED_ENVD_AUTO_UPGRADE=1`: when the
  challenge frame carries a different protocol number, run the upgrade
  before signing, then `execv` the path the daemon was started from,
  captured at startup, with the same argv and environment. The pid, cwd,
  state dir, and daemon key survive, so the reconnect proves the same key
  and lands on the same environment and incarnation; the gateway's
  admission stamps the new build into the row. The trigger is protocol
  mismatch only, never sha drift, so a server hotfix restarts nobody's
  daemon. One automatic attempt per lineage, marked through the
  environment of the exec'd process: a second mismatch exits non-zero as
  today, so a stale document cannot loop.
- No signature is required. The daemon already executes arbitrary
  processes the gateway sends it, so the gateway host is already trusted
  for code execution; a `sha256` served over TLS from that host pins a
  download from anywhere else. A signed document only matters for mirrors
  and stays an open idea.
- No wire change: the daemon derives the URL itself. The registration
  identity model is untouched.

## Acceptance

- A workspace release build of envd registers with a `wss://` gateway and
  serves one data route; the unit test fails on a build that reintroduces
  the default connector; the packaged smoke passes.
- The musl envd runs `--version` and registers from `debian:bullseye`,
  `ubuntu:22.04`, and `alpine:3.20` images.
- Every bundle carries a valid `envd.json` whose checksums match the
  archives and whose `protocolVersion` equals the bundled server's; the
  packaged binary's `--print-build` matches the document.
- With the ls.bot side in place: from only `https://ls.bot`, a script
  fetches `/.well-known/lightspeed-envd`, downloads the matching archive
  without credentials, verifies its sha256, and the binary's
  `--print-build` reports the document's `gitSha` and the gateway's
  `initialize` `envd.protocolVersion`.
- A registered daemon reconnecting with a different build shows the new
  version, sha, and protocol number in its environment row.
- A registered daemon with automatic upgrade enabled follows a gateway
  across a protocol bump without operator action, keeping its environment
  and incarnation; without it, the exit log names the document URL.
- The Harbor adapter's provenance records the server sha and the envd sha,
  and its exclusion list no longer contains the two bullseye tasks.

## Open ideas

- a signed discovery document, so a mirror cannot substitute a daemon;
- a smaller envd for constrained images (feature-gated PTY and job support).

## Non-Goals

- Changing the registration identity model (P148). Optional fields on the
  handshake are additive and in scope.
- Runtime behavior of processes and jobs (P151).
- Publishing server or provider binaries as static builds.
- In-place upgrades of provider-managed VMs; that is ls.bot's rebuild plan.
- Serving the discovery document from the dev stack or the gateway process.

## Implementation Slices

### Slice 1 — Connector and test

- Explicit connector on both `TlsSettings` paths, the provider install
  moved into the library, the no-CA unit test, and the packaged-binary
  smoke; cut a snapshot so production stops needing the single-crate
  workaround.

### Slice 2 — Static target

- Add the musl target to the release, update the hz02 image recipe and the
  Harbor adapter's artifact selection, drop the glibc note from both.
  aarch64 when the cross toolchain is in the image.

### Slice 3 — Discovery and self-identification

- `envd.json` in the bundle with `protocolVersion` and nullable `url`,
  `--print-build`, the `ServerInfo` and `ImplementationInfo` fields, row
  metadata on every admission, and the protocol-bump notice in manifest
  validation; regenerate the API contract. The well-known route and archive
  serving are the deployment's slice in ls.bot (`install-deployment` plus a
  Caddy `handle`).

### Slice 4 — Self-upgrade

- `lightspeed-envd upgrade`, the automatic mode behind
  `LIGHTSPEED_ENVD_AUTO_UPGRADE`, the discovery URL override, and the
  single-attempt guard; document both variables in `docs/variables.md`.
  Live coverage: a registered daemon against a gateway that advertises the
  next protocol number, with a local document naming a binary that speaks
  it.
