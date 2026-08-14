#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

tag="${1:-lightspeed-local}"
source release/metadata.env
git_sha="${LIGHTSPEED_GIT_SHA:-$(git rev-parse HEAD)}"
version="${LIGHTSPEED_RELEASE_VERSION:-$LIGHTSPEED_PRODUCT_VERSION}"
docker build --file release/server.Dockerfile \
  --build-arg "LIGHTSPEED_RELEASE_VERSION=$version" \
  --build-arg "LIGHTSPEED_GIT_SHA=$git_sha" \
  --tag "${tag}-server" .
docker build --file release/configurator-mcp.Dockerfile \
  --build-arg "LIGHTSPEED_RELEASE_VERSION=$version" \
  --build-arg "LIGHTSPEED_GIT_SHA=$git_sha" \
  --tag "${tag}-configurator-mcp" .

container_id="$(docker create "${tag}-server" --version)"
trap 'docker rm -f "$container_id" >/dev/null 2>&1 || true' EXIT
tmp_dir="$(mktemp -d)"
trap 'docker rm -f "$container_id" >/dev/null 2>&1 || true; rm -rf "$tmp_dir"' EXIT
docker cp "$container_id:/usr/local/bin/lightspeed-server" "$tmp_dir/lightspeed-server"
cmp dist/bin/lightspeed-server "$tmp_dir/lightspeed-server"
docker run --rm "${tag}-server" --version
node -e 'JSON.parse(require("node:fs").readFileSync("dist/release-manifest.json", "utf8"))'
