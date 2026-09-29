#!/usr/bin/env bash
# Shared Rust argument construction for the default network/volume entrypoints.
# Functions build argv without expanding secrets into logs or process arguments.
rust_hub_arguments() {
  hub_args=(serve --hub-only --quiet --no-mcp --no-jobs --uploads-to-worker
    --host "${HUB_BIND}" --hub-port "${HUB_PORT}"
    --config-dir "${XDG_CONFIG_HOME}/workspacer"
    --data-dir "${XDG_CONFIG_HOME}/workspacer-hub"
    --plugins-dir "${PLUGINS_DIR}")
  if [ -d /usr/local/share/workspacer/examples ]; then
    hub_args+=(--examples-dir /usr/local/share/workspacer/examples)
  fi
}
rust_worker_arguments() {
  [ -n "${HUB_BUS_URL:-}" ] || { printf '%s\n' 'HUB_BUS_URL is required' >&2; return 1; }
  [ -n "${HUB_TOKEN:-}${WKS_PROVIDER_TOKEN_FILE:-}" ] || { printf '%s\n' 'A separate upstream provider credential is required' >&2; return 1; }
  local scope="${WKS_BRAIN_SCOPE:-full}"
  case "$scope" in full|catalog) ;; *) printf '%s\n' 'WKS_BRAIN_SCOPE must be full or catalog' >&2; return 1 ;; esac
  if [ "$scope" = full ]; then
    [ "${MCP_FACADE_ENABLED:-1}" = 1 ] || { printf '%s\n' 'Full Rust workers require the MCP facade' >&2; return 1; }
    [ -n "${WKS_MCP_HUB_TOKEN:-}${WKS_MCP_HUB_TOKEN_FILE:-}" ] || { printf '%s\n' 'Full Rust workers require a distinct scoped facade caller credential; run provision-worker-caller.sh on the hub' >&2; return 1; }
    case "${WKS_MCP_UNTOKENED:-deny}" in deny|view|operator) ;; *) printf '%s\n' 'Invalid MCP access policy' >&2; return 1 ;; esac
  fi
  if [ -n "${MCP_FACADE_ADDR:-}" ] && [ "$MCP_FACADE_ADDR" != "127.0.0.1:${MCP_FACADE_PORT}" ]; then
    printf '%s\n' 'Rust worker MCP binds loopback; use WKS_MCP_FACADE_PORT without a conflicting address' >&2; return 1
  fi
  worker_args=(serve --quiet --host 127.0.0.1 --hub-port "${WKS_WORKER_HUB_PORT:-7895}"
    --upstream "$HUB_BUS_URL" --provider-scope "$scope" --node-id "${WKS_NODE_ID:-}"
    --config-dir "${XDG_CONFIG_HOME}/workspacer" --home-dir "$WKS_HOME"
    --claudemon-db-path "$WKS_STATE_DB" --claudemon-api-port "$CLAUDEMON_API_PORT"
    --claudemon-hook-port "$CLAUDEMON_HOOK_PORT" --mcp-port "$MCP_FACADE_PORT"
    --untokened "${WKS_MCP_UNTOKENED:-deny}")
  [ -z "${WKS_PROVIDER_TOKEN_FILE:-}" ] || worker_args+=(--upstream-token-file "$WKS_PROVIDER_TOKEN_FILE")
  [ -z "${WKS_MCP_HUB_TOKEN_FILE:-}" ] || worker_args+=(--upstream-caller-token-file "$WKS_MCP_HUB_TOKEN_FILE")
  [ "${MCP_FACADE_ENABLED:-1}" = 1 ] || worker_args+=(--no-mcp)
}
# Only call in a background subshell: exec makes the supervised PID the Rust
# process itself, so the established INT/TERM drain cannot strand a shell child.
rust_as_wks() {
  exec setpriv --reuid="$WKS_UID" --regid="$WKS_GID" --init-groups --inh-caps=-all -- "$@"
}
run_rust_worker() {
  rust_worker_arguments || die 'invalid Rust worker configuration'
  log 'starting owned Rust worker backend (engine, MCP and outbound provider relay)'
  rust_as_wks /usr/local/bin/workspacer-rust "${worker_args[@]}" &
  BRAIN_PID=$! # Existing shutdown trap now owns this one backend PID.
  for _ in $(seq 1 100); do
    curl -sf "${CLAUDEMON_URL}/sessions" >/dev/null 2>&1 && break
    kill -0 "$BRAIN_PID" 2>/dev/null || die 'Rust backend exited during startup'
    sleep 0.2
  done
  curl -sf "${CLAUDEMON_URL}/sessions" >/dev/null 2>&1 || die 'Rust execution engine did not become ready'
  log "BOOT COMPLETE $BOOT_ID; remote readiness still requires authenticated provider registration"
  while true; do
    if ! kill -0 "$BRAIN_PID" 2>/dev/null; then
      wait "$BRAIN_PID" 2>/dev/null || true
      EXIT_REASON=rust-backend-died
      log 'Rust backend exited; bringing worker down so Fly can restart it'
      record_exit 1
      return 1
    fi
    if ! pid_alive "$TAILSCALED_PID"; then
      EXIT_REASON=tailscale-died
      kill -TERM "$BRAIN_PID" 2>/dev/null || true
      record_exit 1
      return 1
    fi
    sleep 5
  done
}
