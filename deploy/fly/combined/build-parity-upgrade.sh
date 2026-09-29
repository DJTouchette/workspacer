#!/usr/bin/env bash
# Build against the CURRENT isolated image. No credentials or live data are read.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'usage: build-parity-upgrade.sh CURRENT_IMAGE OUTPUT_IMAGE' >&2
  exit 2
fi
base=$1 tag=$2
root=$(cd "$(dirname "$0")/../../.." && pwd)
# Upgrade the owned backend and embedded engine together; no legacy child fallback.
exec "$root/deploy/fly/rust/build-upgrade.sh" "$base" "$tag" --isolated
