#!/usr/bin/env bash
set -euo pipefail
export WKS_DATA="${WKS_DATA:-/data}"
export WKS_HOME="${WKS_HOME:-/data/home}"
export HOME="$WKS_HOME"
export XDG_CONFIG_HOME="$WKS_HOME/.config"
export XDG_DATA_HOME="$WKS_HOME/.local/share"
export XDG_STATE_HOME="$WKS_HOME/.local/state"
export XDG_CACHE_HOME="/data/cache/xdg"
umask 077
/usr/local/lib/wks/bootstrap.sh
mkdir -p /data/logs
# Explicit Rust image role: one owned backend; no separate Go/Node companion.
if [ "${WKS_RUST_BACKEND:-0}" = 1 ]; then
  exec setpriv --reuid=wks --regid=wks --init-groups \
    /usr/local/bin/workspacer-rust serve --quiet --host 0.0.0.0 --hub-port 7895 \
    --trusted-host "${HUB_TRUSTED_HOSTS:-${FLY_APP_NAME:?}.fly.dev}" \
    --webapp-dir /usr/local/share/workspacer/web \
    --home-dir "$WKS_HOME" --config-dir "$XDG_CONFIG_HOME/workspacer" \
    --data-dir "$XDG_CONFIG_HOME/workspacer-hub" \
    --claudemon-db-path "${WKS_STATE_DB:-$XDG_DATA_HOME/claudemon/state.db}"
fi
# The CLI's ready banner contains the pairing token. Keep it off Fly logs.
# The rest of the stack logs to stderr as usual.
exec setpriv --reuid=wks --regid=wks --init-groups \
  workspacer serve --host 0.0.0.0 --hub-port 7895 \
  --trusted-host "${HUB_TRUSTED_HOSTS:-${FLY_APP_NAME:?}.fly.dev}" \
  --webapp-dir /usr/local/share/workspacer/web --json \
  >/data/logs/serve-ready.json
