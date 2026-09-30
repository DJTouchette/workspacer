#!/usr/bin/env bash
# Build a replacement image only. No Fly CLI, live mount, secret or cloud write.
set -euo pipefail
[ "$#" -ge 2 ] || { echo 'usage: build-upgrade.sh BASE_IMAGE OUTPUT_IMAGE [--isolated] [--power APP MACHINE observe|stop|off]' >&2; exit 2; }
base=$1 tag=$2
shift 2
isolated=0
power=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --isolated) isolated=1; shift ;;
    --power) [ "$#" -ge 4 ] || exit 2; power=("$2" "$3" "$4"); shift 4 ;;
    *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done
root=$(cd "$(dirname "$0")/../../.." && pwd)
# The stamp must name the actual reviewed source, not an unrelated HEAD plus
# unrecorded local edits. This also prevents shipping untracked build output.
git -C "$root" diff --quiet && git -C "$root" diff --cached --quiet || { echo 'commit reviewed source changes before building an upgrade' >&2; exit 2; }
[ -z "$(git -C "$root" ls-files --others --exclude-standard)" ] || { echo 'untracked source exists; review it before building an upgrade' >&2; exit 2; }
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/bin" "$stage/examples" "$stage/entrypoints"
role=$(docker image inspect --format '{{index .Config.Labels "dev.workspacer.node.role"}}' "$base")
if [ "$isolated" = 1 ]; then
  docker run --rm --entrypoint cat "$base" /opt/combined/supervisor.py >"$stage/supervisor.py"
  python3 "$root/deploy/fly/rust/upgrade-supervisor.py" "$stage/supervisor.py" "$stage/supervisor.py"
  role=isolated
else
  case "$role" in rust-hub) role=hub ;; rust-node) role=node ;; rust-combined) role=combined ;; *) echo 'base is not a reviewed Rust role; use explicit --isolated for the audited protected supervisor' >&2; exit 2 ;; esac
  [ "${#power[@]}" = 0 ] || { echo 'power-file overrides require the isolated hub UID boundary' >&2; exit 2; }
fi
cargo build --locked --release --manifest-path "$root/services/hub-rs/Cargo.toml" --bin workspacer-rust
cargo build --locked --release --manifest-path "$root/services/claudemon/Cargo.toml" --bin claudemon
# Respect a caller's shared target directory rather than copying stale artifacts.
hub_target=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/services/hub-rs/Cargo.toml" | jq -r .target_directory)
engine_target=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/services/claudemon/Cargo.toml" | jq -r .target_directory)
cp "$hub_target/release/workspacer-rust" "$engine_target/release/claudemon" "$stage/bin/"
npm --prefix "$root/apps/desktop" run build:renderer:web
cp -a "$root/apps/desktop/dist/web" "$stage/web"
git -C "$root" archive HEAD plugins/examples | tar -x -C "$stage/examples" --strip-components=2
cp "$root/deploy/fly/rust/launch.sh" "$root/deploy/fly/rust/provision-worker-caller.sh" "$root/deploy/fly/rust/verify-image.sh" "$stage/entrypoints/"
cp "$root/deploy/fly/node/entrypoint.sh" "$stage/node-entrypoint.sh"
cp "$root/deploy/fly/hub/entrypoint.sh" "$stage/hub-entrypoint.sh"
cp "$root/deploy/fly/combined/entrypoint.sh" "$stage/combined-entrypoint.sh"
cp "$root/deploy/fly/rust/image-contract.sh" "$stage/image-contract.sh"
case "$(uname -m)" in x86_64) platform=linux-x64 ;; aarch64) platform=linux-arm64 ;; *) echo 'unsupported upgrade build architecture' >&2; exit 2 ;; esac
WKS_STAMP_COMPONENT=rust-server WKS_STAMP_INSTALL=source WKS_STAMP_COMMIT="$(git -C "$root" rev-parse HEAD)" \
  WKS_STAMP_PLATFORM="$platform" bash "$root/deploy/fly/write-build-stamp.sh" "$stage/build-stamp"
cat >"$stage/Dockerfile" <<'DOCKER'
ARG BASE
FROM ${BASE}
COPY --chmod=0755 bin/ /usr/local/bin/
COPY web/ /usr/local/share/workspacer/web/
COPY examples/ /usr/local/share/workspacer/examples/
COPY --chmod=0755 entrypoints/ /usr/local/lib/wks-rust/
COPY build-stamp /usr/local/share/workspacer/build-stamp
COPY build-stamp /usr/local/share/workspacer/build-stamp.hub
RUN rm -f /usr/local/bin/hub /usr/local/bin/brain /usr/local/bin/mcp /usr/local/bin/workspacer /usr/local/bin/desktop-host.cjs && ln -s workspacer-rust /usr/local/bin/workspacer
DOCKER
if [ "$role" = isolated ]; then
  cat >>"$stage/Dockerfile" <<'DOCKER'
COPY supervisor.py /opt/combined/supervisor.py
RUN python3 -m py_compile /opt/combined/supervisor.py && test -x /usr/local/bin/workspacer-rust && test -x /usr/local/bin/claudemon
LABEL dev.workspacer.node.role="rust-isolated"
DOCKER
  if [ "${#power[@]}" != 0 ]; then
    python3 - "$stage/power.json" "${power[@]}" <<'PY'
import json,pathlib,re,sys
path,app,machine,mode=sys.argv[1:]
if mode not in ('observe','stop','off') or not re.fullmatch(r'[a-z0-9-]+',app) or not re.fullmatch(r'[a-zA-Z0-9]+',machine):
    raise SystemExit('invalid fixed machine power target')
pathlib.Path(path).write_text(json.dumps(dict(app=app,machine=machine,mode=mode,timeout='15m',wakeUrl=f'https://{app}.fly.dev/health')))
PY
    printf '%s\n' 'COPY power.json /opt/combined/power.json' >>"$stage/Dockerfile"
  fi
else
  printf 'COPY --chmod=0755 %s-entrypoint.sh /usr/local/lib/wks%s/entrypoint.sh\n' "$role" "$([ "$role" = node ] || printf -- '-%s' "$role")" >>"$stage/Dockerfile"
  printf 'COPY --chmod=0755 image-contract.sh /usr/local/lib/wks/verify-image.sh\nENV WKS_RUST_BACKEND=1 WKS_RUST_IMAGE_ROLE=%s\nRUN /usr/local/lib/wks-rust/verify-image.sh %s\n' "$role" "$role" >>"$stage/Dockerfile"
fi
docker build --build-arg "BASE=$base" -t "$tag" "$stage"
