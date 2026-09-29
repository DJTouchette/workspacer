#!/usr/bin/env bash
# No cloud deployment. Docker build/boot are explicit local stages.
set -euo pipefail
root=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$root"
stage=${1:-all}
case "$stage" in all|static|build|boot|artifact) ;; *) echo 'usage: preflight.sh [all|static|build|boot|artifact]' >&2; exit 2 ;; esac
if [ "$stage" = all ] || [ "$stage" = static ]; then
  : "${WORKSPACER_RUST_BIN:?set path to the built Rust CLI for static contracts}"
  bash deploy/fly/rust/test-launch.sh
  python3 deploy/fly/rust/test-supervisor.py
  bash deploy/fly/node/test-bootstrap.sh
  bash deploy/fly/hub/test-bootstrap.sh
  bash deploy/fly/test-fetch-release.sh
  bash deploy/fly/rust/test-artifact.sh
  bash deploy/fly/rust/test-provision.sh "${WORKSPACER_RUST_BIN:?set path to the built Rust CLI for the credential contract}"
fi
if [ "$stage" = all ] || [ "$stage" = build ] || [ "$stage" = artifact ]; then
  command -v docker >/dev/null
  install=source
  [ "$stage" != artifact ] || install=artifact
  for role in node hub combined; do
    extra=()
    [ "$install" != artifact ] || extra+=(--build-arg "WKS_RELEASE_TAG=${WKS_RELEASE_TAG:-nightly}" --build-arg "WKS_RELEASE_SHA=${WKS_RELEASE_SHA:-}")
    docker build -f deploy/fly/rust/Dockerfile --build-arg "WKS_ROLE=$role" --build-arg "WKS_INSTALL=$install" \
      --build-arg "WKS_SOURCE_SHA=${WKS_SOURCE_SHA:-unknown}" "${extra[@]}" -t "workspacer-rust-$role:preview" .
  done
fi
if [ "$stage" = all ] || [ "$stage" = boot ]; then
  bash deploy/fly/rust/boot-rehearsal.sh
fi
