#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "$root/launch.sh"
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
has() { local wanted=$1; shift; local argument; for argument in "$@"; do [ "$argument" != "$wanted" ] || return 0; done; return 1; }
HUB_BIND=127.0.0.1 HUB_PORT=7895 XDG_CONFIG_HOME=/fixture/home/.config PLUGINS_DIR=/fixture/plugins
rust_hub_arguments
for flag in --hub-only --quiet --no-mcp --no-jobs --uploads-to-worker; do has "$flag" "${hub_args[@]}" || fail "missing hub boundary $flag"; done
has /fixture/home/.config/workspacer-hub "${hub_args[@]}" || fail 'lost historical hub state directory'
HUB_BUS_URL=wss://hub.example/bus HUB_TOKEN=PRIVATE_PROVIDER WKS_HOME=/fixture/home WKS_STATE_DB=/fixture/state.db
CLAUDEMON_API_PORT=7891 CLAUDEMON_HOOK_PORT=7890 MCP_FACADE_PORT=7897 MCP_FACADE_ADDR=127.0.0.1:7897 MCP_FACADE_ENABLED=1
unset WKS_MCP_HUB_TOKEN WKS_MCP_HUB_TOKEN_FILE || true
if rust_worker_arguments 2>/dev/null; then fail 'full worker accepted provider token as caller'; fi
WKS_MCP_HUB_TOKEN=PRIVATE_CALLER
rust_worker_arguments
for secret in PRIVATE_PROVIDER PRIVATE_CALLER; do ! has "$secret" "${worker_args[@]}" || fail 'secret in process argv'; done
has --upstream "${worker_args[@]}" || fail 'worker does not attach upstream'
! has --allow-new-token "${worker_args[@]}" || fail 'worker silently overrides lost identity'
WKS_MCP_UNTOKENED=allow
if rust_worker_arguments 2>/dev/null; then fail 'invalid untokened policy accepted'; fi
WKS_MCP_UNTOKENED=operator
rust_worker_arguments
has operator "${worker_args[@]}" || fail 'explicit operator policy lost'
WKS_MCP_UNTOKENED=deny MCP_FACADE_ENABLED=0
if rust_worker_arguments 2>/dev/null; then fail 'full worker allowed missing facade'; fi
WKS_BRAIN_SCOPE=catalog
rust_worker_arguments
has --no-mcp "${worker_args[@]}" || fail 'catalog disable flag lost'
WKS_BRAIN_SCOPE=full MCP_FACADE_ENABLED=1 WKS_PROVIDER_TOKEN_FILE=/private/provider WKS_MCP_HUB_TOKEN_FILE=/private/caller
unset HUB_TOKEN WKS_MCP_HUB_TOKEN
rust_worker_arguments
has --upstream-token-file "${worker_args[@]}" && has --upstream-caller-token-file "${worker_args[@]}" || fail 'secret file inputs missing'
bash -n "$root/launch.sh" "$root/../node/entrypoint.sh" "$root/../hub/entrypoint.sh"
if [ -n "${WORKSPACER_RUST_BIN:-}" ]; then
  "$WORKSPACER_RUST_BIN" "${hub_args[@]}" --help >/dev/null
  "$WORKSPACER_RUST_BIN" "${worker_args[@]}" --help >/dev/null
fi
printf '%s\n' 'PASS: Rust container launch boundaries and shell syntax'
