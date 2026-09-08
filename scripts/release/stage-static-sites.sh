#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
source release/metadata.env
version="${LIGHTSPEED_RELEASE_VERSION:-$LIGHTSPEED_PRODUCT_VERSION}"
dist_dir="${1:-$repo_root/dist}"
mkdir -p "$dist_dir/archives"
tar_command=tar
if command -v gtar >/dev/null 2>&1; then
  tar_command=gtar
fi

for site in demo docs; do
  if [[ "$site" = demo ]]; then
    source_dir=platform/web/dist-demo
  else
    source_dir=docs/site/dist
  fi
  if [[ ! -f "$source_dir/index.html" ]]; then
    echo "Missing built $site site: $source_dir/index.html" >&2
    exit 1
  fi
  "$tar_command" --format=gnu --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -C "$source_dir" -cf - . \
    | gzip -n > "$dist_dir/archives/lightspeed-$site-$version.tar.gz"
done
