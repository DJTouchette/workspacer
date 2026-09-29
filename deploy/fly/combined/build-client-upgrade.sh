#!/usr/bin/env bash
# Replace the embedded mobile client and browser assets in an already configured isolated deployment.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'usage: build-client-upgrade.sh BASE_IMAGE OUTPUT_IMAGE' >&2
  exit 2
fi
base=$1 tag=$2
root=$(cd "$(dirname "$0")/../../.." && pwd)
# Upgrade the owned backend and embedded engine together; no legacy child fallback.
exec "$root/deploy/fly/rust/build-upgrade.sh" "$base" "$tag" --isolated
