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

Continuous CI first publishes the composite `build-env` image by source commit,
then runs that exact image by digest. The release manifest records this
composite image digest, not merely the Rust base-image digest.

Release constants are centralized in `release/metadata.env`; run
`scripts/release/verify-metadata.sh` after changing a product, protocol, schema,
toolchain, or build-image version. Every executable reports the product
version, full source commit, target, and Rust version through `--version`.

## Publication

- Pull requests run on GitHub-hosted runners and perform formatting, lint,
  workspace tests, contract checks, both generated-consumer checks, and a
  native Apple Silicon compile/`--version` smoke test. Published standalone
  archives remain Linux-only in the first cut; macOS development uses
  `cargo run`.
- Protected `main` builds run only on the isolated
  `self-hosted,linux,x64,hz01,protected` runner. They publish `sha-<full-sha>`
  images and OCI binary/release bundles, with `edge` as a developer-only image
  alias.
- A `v<product-version>` annotated tag on protected `main` runs
  `release-tag.yml`. It verifies the successful source build, copies OCI
  manifests without rebuilding, publishes the already-packed TypeScript
  client, and creates the GitHub Release.

The `continuous-release` and `official-release` GitHub environments should
protect their respective credentials. Configure `NPM_TOKEN` only on the
official release environment. Production deployment is deliberately outside
these workflows and belongs to the consuming repository.
