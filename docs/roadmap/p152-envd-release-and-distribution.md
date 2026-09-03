# P152 — envd Release and Distribution

**Status**

- Proposed 2026-09-03, from deploying the `harbor` merge to production and
  running the first Harbor evaluation ([P149](p149-harbor-end-to-end-agent-evaluation.md))
  against it. Two findings are fixed or worked around locally and need to
  land properly; the rest is what an orchestrator needs to pick up the right
  `envd` without reading this repository.
- Behavior of the daemon at runtime is [P151](p151-exec-leftover-processes.md);
  this item is about the binary: how it is built, what it links, how it is
  found, and how a running deployment identifies itself.
- Open: further envd ideas to be added here (see "Open ideas").

## Goal

Anyone who needs to place `lightspeed-envd` on a machine, a VM, a container,
or a benchmark sandbox can find the artifact that matches a deployment, verify
it, run it on any Linux image, and prove which build they ran.

## Problems observed

1. **The release binary panics on its first TLS connection.** The release is
   one workspace build; feature unification compiles both rustls providers
   (`ring` through `object_store`, `aws-lc-rs` through envd's own deps), so
   rustls has no process default and tokio-tungstenite's default connector
   panics with `Could not automatically determine the process-level
   CryptoProvider`. A single-crate `cargo build -p environment-daemon` only
   sees `aws-lc-rs` and works, which is why local builds, the unit suite, and
   the live registration suite never showed it. Every Harbor trial on the
   release binary failed before registering.
2. **glibc pins the sandbox floor.** The release builds on
   `rust:1.97.1-bookworm` and links glibc 2.36. Terminal-Bench images on
   `debian:bullseye` (glibc 2.31) fail `envd --version`; two of 89 tasks had
   to be excluded. Any customer image older than bookworm has the same
   problem.
3. **Nothing points at "the current build".** The deployed commit is only in
   hz01's `/var/lib/ls-deploy/last-good.env`; the envd archive is an OCI
   bundle reachable only through `infra/scripts/fetch-lightspeed-binary`
   with registry credentials; an orchestrator elsewhere has no way to ask
   "which envd matches this gateway" and download it.
4. **The server does not say what it is.** `initialize` returns
   `serverInfo.version: "0.1.0"`; the build's git sha
   (`release_info::GIT_SHA`) is not exposed, so provenance records cannot
   pin the deployment they ran against, and a mismatched envd cannot be
   detected before registration.

## Decision

1. `envd` installs its rustls provider once at startup
   (`rustls::crypto::aws_lc_rs::default_provider().install_default()` in
   `main`), and a workspace-build test opens a TLS client so the class of
   defect cannot return.
2. The published `envd` is a static binary: `x86_64-unknown-linux-musl` and
   `aarch64-unknown-linux-musl`, built with the same pinned toolchain image.
   The glibc build stops being published for envd; server, provider, and CLI
   keep their targets.
3. Every release and `main` snapshot publishes a small discovery document
   inside the bundle. Serving it is the deployment's job, not the gateway's:
   ls.bot publishes the document of the release it installed at a
   well-known path through Caddy, next to the site and demo artifacts it
   already serves by digest. An orchestrator goes from the deployment's
   public host to a verified envd without credentials or a checkout, and
   checks it against the build the gateway reports about itself.
4. `initialize` reports the build: `serverInfo.version` becomes the release
   version plus git sha, and `serverInfo` gains `gitSha`, `builtAtMs`, and
   `envd: {version, gitSha, targets, sha256s, url}` describing the daemon
   that matches this server.

## Design

### Provider selection and the workspace test

- The one-line change in `crates/environment-daemon/src/main.rs`.
- A test in `crates/environment-daemon` that builds a `rustls::ClientConfig`
  through the same path `TlsSettings::from_config(None)` and the data-route
  dial take, run by CI in a workspace-wide build (`cargo test --workspace`),
  not only `-p environment-daemon`.
- `scripts/release/smoke.sh` runs the packaged envd once against a local
  TLS listener (a self-signed acceptor is enough) so the packaged binary,
  not a rebuilt one, is what passes.

### Static targets

- Add `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` to
  `release/metadata.env` as the envd targets; `build-dist.sh` builds envd for
  both and packages `lightspeed-envd-<version>-<target>.tar.gz` each with a
  checksum. `aws-lc-rs` builds on musl with the pinned image's cmake and
  clang; if it does not, `ring` is the fallback provider for the envd crate
  only.
- The Incus image recipe on hz02 and the Harbor adapter consume the musl
  archive; the glibc floor disappears from both documents.

### Discovery

- Each bundle gains `envd.json`: `{version, gitSha, channel: release|main,
  builtAtMs, artifacts: {<target>: {url, sha256}}}` where `url` is a plain
  HTTPS download (GitHub release asset for tags, the snapshot's public
  artifact URL for `main`). Producing it is the release pipeline's part
  (`scripts/release/create-manifest.mjs` alongside `release-manifest.json`);
  `validate-release-manifest` checks it.
- Serving it belongs to the deployment. In ls.bot, `install-deployment`
  copies the installed bundle's `envd.json` under the versions directory
  and Caddy serves it at `GET /.well-known/lightspeed-envd` as a static
  file, switched atomically with the release like `/demo/`. The gateway
  and the API do not know about the path. That is an ls.bot change,
  tracked there; this item only guarantees the document exists in every
  bundle with stable download URLs.
- `lightspeed-envd --print-build` prints its own `{version, gitSha,
  target, sha256}` so an orchestrator can compare the downloaded binary
  with what `initialize` reports before starting it.

### Self-identification

- `ServerInfo` gains `gitSha`, `builtAtMs`, and the `envd` block above;
  `version` becomes `0.1.0+2093b949` style. Clients that only read `name`
  and `version` are unaffected.
- The registration handshake already carries the protocol version; the
  gateway additionally logs the daemon's reported git sha on admission so a
  mismatched daemon is visible in the environment row (`metadata`
  `lightspeed.envdGitSha`, set by the gateway, not the client).

## Acceptance

- A workspace release build of envd registers with a `wss://` gateway and
  serves one data route; the CI test fails on a build without the provider
  call.
- The musl envd runs `--version` and registers from `debian:bullseye`,
  `ubuntu:22.04`, and `alpine:3.20` images.
- Every bundle carries a valid `envd.json` whose download URLs resolve
  without credentials and whose checksums match the archives; the packaged
  binary's `--print-build` matches the manifest.
- With the ls.bot side in place: from only `https://ls.bot`, a script
  fetches `/.well-known/lightspeed-envd`, downloads the matching archive,
  verifies its sha256, and the binary's `--print-build` git sha equals the
  gateway's `initialize` `gitSha`.
- The Harbor adapter's provenance shows matching git shas for server and
  envd, and its exclusion list no longer contains the two bullseye tasks.

## Open ideas

To be filled in; candidates raised so far:

- an envd self-update or "restart into the version the gateway expects"
  path for long-lived persistent environments;
- a signed discovery document, so a compromised mirror cannot substitute a
  daemon;
- a smaller envd for constrained images (feature-gated PTY and job support).

## Non-Goals

- Changing the registration protocol or identity model (P148).
- Runtime behavior of processes and jobs (P151).
- Publishing server or provider binaries as static builds.

## Implementation Slices

### Slice 1 — Provider and test

- Land the `main.rs` change, the workspace TLS test, and the packaged-binary
  smoke; cut a snapshot so production stops needing the single-crate
  workaround.

### Slice 2 — Static targets

- Add the musl targets to the release, update the hz02 image recipe and the
  Harbor adapter's artifact selection, drop the glibc note from both.

### Slice 3 — Discovery and self-identification

- `envd.json` in the bundle, `--print-build`, and the `ServerInfo` fields;
  regenerate the contract. The well-known route is the deployment's slice
  in ls.bot (`install-deployment` plus a Caddy `handle`).
