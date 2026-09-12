#!/usr/bin/env bash
# Replace only browser assets in an already configured isolated deployment.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'usage: build-web-upgrade.sh BASE_IMAGE OUTPUT_IMAGE' >&2
  exit 2
fi
base=$1 tag=$2
root=$(cd "$(dirname "$0")/../../.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
(cd "$root/apps/desktop" && npm run build:renderer:web)
cp -a "$root/apps/desktop/dist/web" "$stage/web"
git -C "$root" rev-parse HEAD >"$stage/web-source"
git -C "$root" diff --binary -- apps/desktop/src/renderer >>"$stage/web-source"
cat >"$stage/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
RUN rm -rf /usr/local/share/workspacer/web
COPY web/ /usr/local/share/workspacer/web/
COPY web-source /usr/local/share/workspacer/web-source
RUN test -f /usr/local/share/workspacer/web/index.html
DOCKER
docker build --build-arg "BASE=$base" -t "$tag" "$stage"
