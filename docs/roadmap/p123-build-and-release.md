# P123 — Lightspeed-owned build and release

Status: Lightspeed repository implementation complete; infrastructure and
consumer cutover remain in ls.bot.

Implemented here:

- digest-pinned Debian 12/Rust 1.93.1 release build environment, published and
  recorded by its composite image digest;
- one Cargo release compilation for server, provider, envd, and CLI artifacts;
- shared executable version/source metadata;
- embedded advisory-locked PostgreSQL migrations with an immutable checksum
  ledger, explicit `migrate`, startup verification, and schema diagnostics;
- deterministic standalone archives, checksums, release manifest schema, SPDX
  SBOM, and artifact smoke tests;
- server and Configurator runtime images that consume prebuilt `dist/` output;
- publishable `@lightspeed/agent-client` release metadata; and
- protected-main immutable publication plus no-rebuild SemVer promotion
  workflows; and
- an Apple Silicon macOS CI build/smoke guard, while macOS release archives
  remain deferred.

Remaining outside this repository:

- provision and harden the hz01 build VM/runner carrying the workflow labels;
- configure GitHub release environments and npm publication credentials;
- pin one resulting manifest in ls.bot and remove its sibling-source build;
- complete the deployment/migration/rollback drill; and
- retire the hz02 CI guest after the required acceptance runs.
