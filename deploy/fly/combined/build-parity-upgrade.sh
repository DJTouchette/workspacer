#!/usr/bin/env bash
# Build against the CURRENT isolated image. No credentials or live data are read.
set -euo pipefail
if [ "$#" -ne 2 ]; then
  echo 'usage: build-parity-upgrade.sh CURRENT_IMAGE OUTPUT_IMAGE' >&2
  exit 2
fi
base=$1 tag=$2
root=$(cd "$(dirname "$0")/../../.." && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/examples"
CGO_ENABLED=0 go -C "$root/services/hub" build -trimpath -o "$stage/bin/" ./cmd/hub ./cmd/workspacer ./cmd/brain ./cmd/mcp
cargo build --release --manifest-path "$root/services/claudemon/Cargo.toml"
cp "$root/services/claudemon/target/release/claudemon" "$stage/bin/claudemon"
npm --prefix "$root/apps/desktop" run build:desktop-host
npm --prefix "$root/apps/desktop" run build:renderer:web
cp "$root/apps/desktop/dist/headless/desktop-host.cjs" "$stage/bin/desktop-host.cjs"
cp -a "$root/apps/desktop/dist/web" "$stage/web"
# Only tracked example assets belong in a distributable image.
python3 - "$root" "$stage" <<'PY'
from pathlib import Path
import shutil, subprocess, sys
root, stage = map(Path, sys.argv[1:])
for item in subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z', 'services/hub/examples']).split(b'\0'):
    if not item:
        continue
    source = root / item.decode()
    target = stage / 'examples' / source.relative_to(root / 'services/hub/examples')
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
PY
docker run --rm --entrypoint cat "$base" /opt/combined/supervisor.py > "$stage/supervisor.py"
python3 "$root/deploy/fly/combined/upgrade-parity-supervisor.py" "$stage/supervisor.py"
cp "$root/deploy/fly/combined/network-admin.py" "$stage/network-admin.py"
python3 -m py_compile "$stage/supervisor.py" "$stage/network-admin.py"
cat > "$stage/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
COPY bin/ /usr/local/bin/
COPY web/ /usr/local/share/workspacer/web/
COPY examples/ /usr/local/share/workspacer/examples/
COPY supervisor.py network-admin.py /opt/combined/
RUN node -e 'const [a,b]=process.versions.node.split(".").map(Number); if(a<22||(a===22&&b<13))process.exit(1); require("node:sqlite")' \
 && python3 -m py_compile /opt/combined/supervisor.py /opt/combined/network-admin.py \
 && test -f /usr/local/share/workspacer/examples/editor/ui/cm.bundle.js \
 && chmod 0755 /usr/local/bin/claudemon /usr/local/bin/hub /usr/local/bin/brain /usr/local/bin/mcp /usr/local/bin/workspacer
DOCKER
docker build --build-arg "BASE=$base" -t "$tag" "$stage"
