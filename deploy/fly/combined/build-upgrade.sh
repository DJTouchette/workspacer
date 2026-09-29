#!/usr/bin/env bash
# Upgrade hub/brain/CLI/client code, preserving the worker daemon image and its
# isolated supervisor. No credentials or live volume contents enter this build.
set -euo pipefail
if [ "$#" -ne 5 ]; then
  echo 'usage: build-upgrade.sh BASE_IMAGE OUTPUT_IMAGE APP MACHINE observe|stop' >&2
  exit 2
fi
base=$1 tag=$2 app=$3 machine=$4 mode=$5
root=$(cd "$(dirname "$0")/../../.." && pwd)
# Upgrade the owned backend and embedded engine together; no legacy child fallback.
exec "$root/deploy/fly/rust/build-upgrade.sh" "$base" "$tag" --isolated --power "$app" "$machine" "$mode"
