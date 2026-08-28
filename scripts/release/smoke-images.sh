#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

platform_image="${PLATFORM_IMAGE:?PLATFORM_IMAGE is required}"
configurator_image="${CONFIGURATOR_IMAGE:?CONFIGURATOR_IMAGE is required}"
platform_workers_image="${PLATFORM_WORKERS_IMAGE:?PLATFORM_WORKERS_IMAGE is required}"
expected_sha="${EXPECTED_SHA:?EXPECTED_SHA is required}"

docker pull "$platform_image"
docker pull "$configurator_image"
docker pull "$platform_workers_image"

test "$(docker image inspect "$platform_image" \
  --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}')" = "$expected_sha"

platform_container="$(docker run --detach --network host \
  -e PORT=18300 \
  -e LIGHTSPEED_PLATFORM_DATABASE_URL="${PLATFORM_DATABASE_URL:?PLATFORM_DATABASE_URL is required}" \
  -e LIGHTSPEED_PLATFORM_AUTH_SECRET=lightspeed-release-smoke-auth-secret \
  -e LIGHTSPEED_PLATFORM_BASE_URL=http://127.0.0.1:18300 \
  "$platform_image")"
cleanup() {
  docker rm -f "$platform_container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

ready=false
for _ in $(seq 1 30); do
  if curl --fail --silent http://127.0.0.1:18300/health \
    | grep -F '"ok":true' >/dev/null; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "$ready" != true ]]; then
  docker logs "$platform_container" >&2
  echo "platform image did not become healthy" >&2
  exit 1
fi
curl --fail --silent http://127.0.0.1:18300/app \
  | grep -F '<div id="root"></div>' >/dev/null

test "$(docker image inspect "$configurator_image" \
  --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}')" = "$expected_sha"
docker run --rm --entrypoint node "$configurator_image" \
  --input-type=module -e 'await import("/app/dist/index.js")'

test "$(docker image inspect "$platform_workers_image" \
  --format '{{ index .Config.Labels "org.opencontainers.image.revision" }}')" = "$expected_sha"
test "$(docker image inspect "$platform_workers_image" --format '{{json .Config.Cmd}}')" = '["all"]'
docker run --rm --entrypoint node "$platform_workers_image" -e \
  'require("node:fs").accessSync("/app/node_modules/baileys/package.json")'
docker run --rm --entrypoint node "$platform_workers_image" --import tsx --input-type=module -e \
  'await import("@lightspeed/bots/webhooks")'
docker run --rm --entrypoint node "$platform_workers_image" -e \
  'require("node:fs").accessSync("/app/platform/workers/src/main.ts")'

for spec in \
  channels-workflows:channels-workflows \
  channels-activities:channels-activities \
  bots-workflows:bots-workflows \
  bots-activities:bots-activities \
  telegram:telegram \
  whatsapp:whatsapp \
  channels:channels-workflows,channels-activities \
  bots:bots-workflows,bots-activities \
  all:channels-workflows,channels-activities,bots-workflows,bots-activities; do
  role="${spec%%:*}"
  expected="${spec#*:}"
  docker run --rm --entrypoint node -e "TEST_ROLE=$role" -e "TEST_EXPECTED=$expected" \
    "$platform_workers_image" \
    --import tsx --input-type=module -e '
      const { resolvePlatformWorkerRoles } = await import("./platform/workers/src/roles.ts");
      const actual = resolvePlatformWorkerRoles(process.env.TEST_ROLE, undefined).join(",");
      if (actual !== process.env.TEST_EXPECTED) process.exit(1);
    '
done
