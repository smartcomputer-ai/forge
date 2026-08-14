#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

if [[ "$#" -ne 1 ]]; then
  echo "usage: run-build-env.sh <digest-pinned-build-image-or-local-tag>" >&2
  exit 2
fi
build_image="$1"
manifest_build_image="${LIGHTSPEED_RELEASE_BUILD_IMAGE:-$build_image}"
if [[ ! "$manifest_build_image" =~ @sha256:[0-9a-f]{64}$ ]]; then
  echo "LIGHTSPEED_RELEASE_BUILD_IMAGE must be digest-pinned" >&2
  exit 1
fi

docker run --rm --platform linux/amd64 \
  -e LIGHTSPEED_RELEASE_VERSION="${LIGHTSPEED_RELEASE_VERSION:-}" \
  -e LIGHTSPEED_GIT_SHA="${LIGHTSPEED_GIT_SHA:-$(git rev-parse HEAD)}" \
  -e LIGHTSPEED_RELEASE_BUILD_IMAGE="$manifest_build_image" \
  -e SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git show -s --format=%ct HEAD)}" \
  -v lightspeed-cargo-registry:/usr/local/cargo/registry \
  -v lightspeed-release-target:/workspace/target \
  -v "$(pwd):/workspace" "$build_image" \
  bash -c '
    cleanup() {
      status=$?
      trap - EXIT
      workspace_owner="$(stat -c %u:%g /workspace)"
      find /workspace -path /workspace/target -prune -o \
        -exec chown "$workspace_owner" {} + || {
        cleanup_status=$?
        if (( status == 0 )); then
          status=$cleanup_status
        fi
      }
      exit "$status"
    }
    trap cleanup EXIT
    scripts/release/build-dist.sh
  '
