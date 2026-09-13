#!/usr/bin/env bash
# Replace claudemon while preserving the deployed supervisor, clients, and policy.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'usage: build-daemon-upgrade.sh BASE_IMAGE OUTPUT_IMAGE' >&2
  exit 2
fi
base=$1 tag=$2
root=$(cd "$(dirname "$0")/../../.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cargo build --release --manifest-path "$root/services/claudemon/Cargo.toml"
cp "$root/services/claudemon/target/release/claudemon" "$stage/claudemon"
cat > "$stage/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
COPY --chown=0:0 --chmod=0755 claudemon /usr/local/bin/claudemon
DOCKER
docker build --build-arg "BASE=$base" -t "$tag" "$stage"
