# Build and release

Lightspeed owns and publishes a coherent release containing the hosted server,
the Incus provider, envd, the CLI, Configurator MCP, the generated TypeScript
client, API contracts, checksums, an SPDX SBOM, and a release manifest. A
consumer should pin one manifest rather than selecting components separately.

## Database migrations

PostgreSQL migrations are embedded in `lightspeed-server`. Apply them before
starting any gateway or worker process:

```bash
lightspeed-server migrate
lightspeed-server schema-version
lightspeed-server both
```

The migrate command uses `LIGHTSPEED_POSTGRES_URL` (falling back to the test
URL only in development), takes a PostgreSQL advisory lock, verifies immutable
SHA-256 checksums in `schema_migrations`, and applies each pending file in its
own transaction. Normal startup only verifies the ledger and exits with a
diagnostic when migration is required.

An existing Lightspeed schema without ledger entries is never baselined
automatically. By default, both startup verification and `migrate` fail and
list the recognized tables. Upgrading such a pre-ledger database requires an
explicit, validated adoption procedure; until one exists, reset the full
Lightspeed schema or database before applying the embedded migrations.
Resetting only the environment tables is insufficient.

Deployments that deliberately provision the Lightspeed tables through an
external schema-management system can set:

```bash
LIGHTSPEED_ALLOW_UNLEDGERED_SCHEMA=true
```

This permits gateway and worker startup only when Lightspeed detects its tables
without ledger entries. It emits a warning and makes the operator responsible
for schema compatibility. It does not fabricate migration records, bypass a
stale or corrupted ledger, or make `lightspeed-server migrate` accept an
unledgered database.

Never edit a migration after an official release. Add the next contiguous SQL
file under `crates/store-pg/migrations/`, register it in
`crates/store-pg/src/migrations.rs`, and bump `LIGHTSPEED_SCHEMA_REVISION` in
`release/metadata.env`.

## Local release build

The authoritative build runs inside the digest-pinned Debian 12/Rust image:

```bash
make release
```

`make release-dist` compiles all Rust executables in one Cargo invocation and
produces `dist/`. `make release-images` copies those prebuilt files into the
two runtime images; it does not invoke Cargo. The image smoke test extracts the
server executable and compares it byte-for-byte with `dist/bin`.

Release and snapshot builds first publish the composite `build-env` image under
a run-specific staging tag, then run that exact image by digest. The release
manifest records this composite image digest, not merely the Rust base-image
digest.

Release constants are centralized in `release/metadata.env`; run
`scripts/release/verify-metadata.sh` after changing a product, protocol, schema,
toolchain, or build-image version. Every executable reports the product
version, full source commit, target, and Rust version through `--version`.

## Publication

- Pull requests and pushes to `main` run formatting, lint, cached workspace
  tests, contract checks, both generated-consumer checks, and the live
  migration-ledger acceptance test on GitHub-hosted runners. They publish
  nothing.
- `.github/workflows/macos.yml` provides a manual native Apple Silicon
  compile/`--version` smoke test. Published standalone archives remain
  Linux-only in the first cut; macOS development uses `cargo run`.
- A successful push-to-`main` run of `ci.yml` triggers
  `.github/workflows/snapshot-main.yml`. It checks out the exact SHA reported by
  CI, confirms that it is still the head of `origin/main`, and builds one
  coherent Linux artifact set on hz01 without repeating the CI test suite.
- Snapshot components are first published under a run-specific staging tag and
  recorded by digest in the manifest. After package, archive, manifest, image,
  checksum, and binary/image identity checks pass, the workflow rechecks the
  head of `main` and assigns `release-bundle:sha-<full-sha>` as the single
  public snapshot identity. Consumers resolve that tag once and follow only the
  digest-pinned component references in its manifest. A superseded or canceled
  run may leave staging objects but cannot expose a complete snapshot.
- The ls.bot notification/dispatch step is intentionally not wired yet. For
  now, a completed `release-bundle:sha-<full-sha>` is the output that ls.bot can
  consume later by digest.
- A `v<product-version>` annotated tag on `main` triggers
  `.github/workflows/release-tag.yml`. It independently tests and builds the
  exact tagged commit, applies SemVer aliases from the manifest's exact
  digests, publishes the stable TypeScript client, and creates the GitHub
  Release. Release versions may use prerelease suffixes but not `+build`
  metadata because the same version is also an OCI tag. The workflow never
  looks up or promotes a prior main snapshot.

The `official-release` GitHub environment protects tagged-release credentials;
configure `NPM_TOKEN` only there. Snapshot publication uses only the scoped
GitHub token. Deployment remains outside these workflows until the explicit
ls.bot notification is added.
