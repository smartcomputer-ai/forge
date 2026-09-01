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
  if [[ "$archive" != *-demo-* ]]; then
    [[ "$(tar -tzf "$archive" | wc -l)" -eq 1 ]]
  fi
done

demo_tgz="$(find dist/archives -maxdepth 1 -name '*-demo-*' -print -quit)"
test -n "$demo_tgz"
tar -tzf "$demo_tgz" ./index.html >/dev/null
tar -tzf "$demo_tgz" ./favicon.svg >/dev/null
demo_files="$(mktemp)"
trap 'rm -f "$demo_files"' EXIT
tar -tzf "$demo_tgz" > "$demo_files"
grep -Eq '^\./assets/.+\.(css|js)$' "$demo_files"

client_tgz="$(find dist/npm -maxdepth 1 -name '*.tgz' -print -quit)"
for entry in package/package.json package/release.json package/dist/index.js \
  package/schema/api.schema.json; do
  tar -tzf "$client_tgz" "$entry" >/dev/null
done
test -f dist/configurator-mcp/dist/bin.js
test -f dist/configurator-mcp/node_modules/@lightspeed-ai/agent-client/dist/index.js

for runtime in platform platform-workers; do
  test -f "dist/runtime/$runtime.tar.gz"
  tar -tzf "dist/runtime/$runtime.tar.gz" ./package.json >/dev/null
done
tar -tzf dist/runtime/platform.tar.gz ./platform/server/src/main.ts >/dev/null
tar -tzf dist/runtime/platform.tar.gz ./platform/web/dist/index.html >/dev/null
tar -tzf dist/runtime/platform-workers.tar.gz ./platform/connectors/src/host/main.ts >/dev/null
tar -tzf dist/runtime/platform-workers.tar.gz ./platform/connectors/src/providers/telegram/connector.ts >/dev/null
tar -tzf dist/runtime/platform-workers.tar.gz ./platform/connectors/src/providers/whatsapp/connector.ts >/dev/null
platform_worker_files="$(mktemp)"
trap 'rm -f "$demo_files" "$platform_worker_files"' EXIT
tar -tzf dist/runtime/platform-workers.tar.gz > "$platform_worker_files"
grep -Eq '^\./node_modules/baileys/' "$platform_worker_files"

(cd dist && sha256sum --check checksums.txt)
scripts/release/verify-manifest.mjs
