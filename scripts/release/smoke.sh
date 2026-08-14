#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

expected_sha="${LIGHTSPEED_GIT_SHA:-$(git rev-parse HEAD)}"
for binary in dist/bin/lightspeed-server dist/bin/lightspeed-provider-incus \
  dist/bin/lightspeed-envd dist/bin/lightspeed; do
  version_output="$($binary --version)"
  grep -F "$expected_sha" <<<"$version_output" >/dev/null
done

for archive in dist/archives/*.tar.gz; do
  [[ "$(tar -tzf "$archive" | wc -l)" -eq 1 ]]
done

client_tgz="$(find dist/npm -maxdepth 1 -name '*.tgz' -print -quit)"
for entry in package/package.json package/release.json package/dist/index.js \
  package/schema/api.schema.json; do
  tar -tzf "$client_tgz" "$entry" >/dev/null
done
test -f dist/configurator-mcp/dist/bin.js
test -f dist/configurator-mcp/node_modules/@lightspeed/agent-client/dist/index.js

(cd dist && sha256sum --check checksums.txt)
scripts/release/verify-manifest.mjs
