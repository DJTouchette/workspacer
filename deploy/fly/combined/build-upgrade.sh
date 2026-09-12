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
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/bin"
(cd "$root/services/hub" && CGO_ENABLED=0 go build -trimpath -o "$stage/bin/" ./cmd/hub ./cmd/workspacer ./cmd/brain)
(cd "$root/apps/desktop" && npm run build:renderer:web)
cp -a "$root/apps/desktop/dist/web" "$stage/web"
docker run --rm --entrypoint cat "$base" /opt/combined/supervisor.py >"$stage/supervisor.py"
docker run --rm --entrypoint cat "$base" /opt/combined/entry.sh >"$stage/entry.sh"
python3 "$root/deploy/fly/combined/upgrade-supervisor.py" "$stage/supervisor.py"
python3 - "$stage" "$app" "$machine" "$mode" <<'PY'
import json, pathlib, sys
stage, app, machine, mode = sys.argv[1:]
entry = pathlib.Path(stage, 'entry.sh')
s = entry.read_text()
assert s.count('/usr/bin/env -i PATH=') == 1
entry.write_text(s.replace('/usr/bin/env -i PATH=', '/usr/bin/env -i FLY_APP_NAME="$FLY_APP_NAME" FLY_MACHINE_ID="$FLY_MACHINE_ID" PATH='))
assert mode in ('observe', 'stop')
pathlib.Path(stage, 'power.json').write_text(json.dumps(dict(app=app, machine=machine, mode=mode, timeout='15m', wakeUrl=f'https://{app}.fly.dev/health')))
PY
git -C "$root" rev-parse HEAD >"$stage/power-source"
git -C "$root" diff --binary >>"$stage/power-source"
sha256sum "$stage/bin/hub" "$stage/bin/workspacer" "$stage/bin/brain" >"$stage/power-binaries"
cat >"$stage/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
COPY bin/ /usr/local/bin/
COPY web/ /usr/local/share/workspacer/web/
COPY supervisor.py power.json entry.sh /opt/combined/
COPY power-source power-binaries /usr/local/share/workspacer/
RUN chmod 0755 /opt/combined/entry.sh && python3 -m py_compile /opt/combined/supervisor.py && test -x /usr/local/bin/hub && test -f /usr/local/share/workspacer/web/index.html
DOCKER
docker build --build-arg "BASE=$base" -t "$tag" "$stage"
