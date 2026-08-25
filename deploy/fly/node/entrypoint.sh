#!/usr/bin/env bash
#
# entrypoint.sh — PID 1 on the Fly worker node.
#
# Topology is PROVIDER-ATTACH, not federation. This machine runs exactly two
# workspacer processes:
#
#   claudemon serve --host 127.0.0.1        the session daemon, loopback only
#   brain --hub <wss://…/bus> --token <tok> dials the always-on hub and
#                                           registers ~60 capabilities
#
# There is no hub on this machine, no peers.json, no federation link. From the
# always-on hub's point of view this node's sessions are ordinary local
# sessions, served by a provider that happens to be a thousand miles away.
#
# Boot order and why:
#   1. boot log on the volume        Fly keeps logs 7 days; this machine may
#                                    sleep for weeks. A boot failure from three
#                                    weeks ago has to be recoverable.
#   2. bootstrap the volume          refuses to run if /data is not mounted
#   3. doorbell                      binds early so a Fly-proxy autostart request
#                                    is answered instead of timing out
#   4. tailscaled + tailscale up     started before claudemon so its reconnect
#                                    overlaps the rest of the boot
#   5. wait for the tailnet          the brain must not dial the hub before the
#                                    tailnet is up, or it burns its first dial
#   6. claudemon init                installs the hook + statusLine forwarder
#                                    into ~/.claude/settings.json. NOTHING in
#                                    the serve path ever runs this; without it
#                                    PTY sessions emit no hook events and read
#                                    as permanently idle. Idempotent.
#   7. claudemon serve, then brain
#
set -euo pipefail

# --------------------------------------------------------------------------
# Environment. fly.toml [env] wins; these are the defaults so the image also
# runs under a plain `docker run`.
# --------------------------------------------------------------------------
export WKS_DATA="${WKS_DATA:-/data}"
export WKS_HOME="${WKS_HOME:-${WKS_DATA}/home}"
export WKS_UID="${WKS_UID:-10001}"
export WKS_GID="${WKS_GID:-10001}"
export WKS_USER="${WKS_USER:-wks}"

export HOME="$WKS_HOME"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$WKS_HOME/.config}"
export XDG_DATA_HOME="${XDG_DATA_HOME:-$WKS_HOME/.local/share}"
export XDG_STATE_HOME="${XDG_STATE_HOME:-$WKS_HOME/.local/state}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$WKS_DATA/cache/xdg}"

export GOPATH="${GOPATH:-$WKS_DATA/go}"
export GOMODCACHE="${GOMODCACHE:-$WKS_DATA/go/pkg/mod}"
export GOCACHE="${GOCACHE:-$WKS_DATA/go/cache}"
export BUNDLE_PATH="${BUNDLE_PATH:-$WKS_DATA/bundle}"
export BUN_INSTALL_CACHE_DIR="${BUN_INSTALL_CACHE_DIR:-$WKS_DATA/bun}"
export npm_config_cache="${npm_config_cache:-$WKS_DATA/npm}"
export HISTFILE="${HISTFILE:-$WKS_HOME/.bash_history}"
export PATH="/usr/local/go/bin:$GOPATH/bin:$PATH"

# claudemon's SQLite store. Passed EXPLICITLY rather than left to
# default_db_path()'s fallback chain, whose third arm is a RELATIVE path under
# the process CWD — i.e. the ephemeral rootfs — if HOME is ever unset. The
# explicit path is the same one the XDG default resolves to, so a hand-run
# claudemon finds the same database.
export WKS_STATE_DB="${WKS_STATE_DB:-$XDG_DATA_HOME/claudemon/state.db}"

CLAUDEMON_API_PORT="${CLAUDEMON_API_PORT:-7891}"
CLAUDEMON_HOOK_PORT="${CLAUDEMON_HOOK_PORT:-7890}"
CLAUDEMON_URL="http://127.0.0.1:${CLAUDEMON_API_PORT}"

TS_STATE="${WKS_DATA}/tailscale/tailscaled.state"
TS_SOCKET="${TS_SOCKET:-/var/run/tailscale/tailscaled.sock}"
TS_HOSTNAME="${WKS_TS_HOSTNAME:-${FLY_APP_NAME:-workspacer-node}}"
TS_WAIT_SECS="${WKS_TAILNET_WAIT_SECS:-60}"

DOORBELL_PORT="${WKS_DOORBELL_PORT:-8080}"
DOORBELL_ENABLED="${WKS_DOORBELL_ENABLED:-1}"

BOOT_LOG="${WKS_DATA}/logs/boot.log"
LAST_BOOT_LOG="${WKS_DATA}/logs/last-boot.log"
BOOT_LOG_MAX_BYTES="${WKS_BOOT_LOG_MAX_BYTES:-5242880}" # 5 MiB
BOOT_ID="$(date -u +%Y%m%dT%H%M%SZ)-${FLY_MACHINE_ID:-local}"

# --------------------------------------------------------------------------
# 1. The boot log, before anything else runs.
# --------------------------------------------------------------------------
# Fly's log retention is 7 days and this machine is designed to be off most of
# the time, so the platform's logs cannot be the record of why a wake failed.
# Everything from here on is tee'd to the volume as well as to stdout.
mkdir -p "${WKS_DATA}/logs" 2>/dev/null || true

if [ -f "$BOOT_LOG" ]; then
  sz="$(wc -c <"$BOOT_LOG" 2>/dev/null || echo 0)"
  if [ "$sz" -gt "$BOOT_LOG_MAX_BYTES" ]; then
    tail -c $((BOOT_LOG_MAX_BYTES / 2)) "$BOOT_LOG" >"${BOOT_LOG}.trim" 2>/dev/null &&
      mv "${BOOT_LOG}.trim" "$BOOT_LOG"
  fi
fi

: >"$LAST_BOOT_LOG" 2>/dev/null || true
exec > >(tee -a "$BOOT_LOG" "$LAST_BOOT_LOG") 2>&1

log() { printf '%s entrypoint: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }

echo "================================================================"
log "BOOT $BOOT_ID"
log "  app=${FLY_APP_NAME:-<none>} machine=${FLY_MACHINE_ID:-<none>} region=${FLY_REGION:-<none>} image=${FLY_IMAGE_REF:-<none>}"
if [ -f "${WKS_DATA}/state/last-exit.json" ]; then
  log "  previous run ended: $(cat "${WKS_DATA}/state/last-exit.json")"
else
  log "  previous run ended: <no record — first boot, or the volume was not mounted last time>"
fi

# --------------------------------------------------------------------------
# Record why this run ended, so "stopped" is not ambiguous.
# --------------------------------------------------------------------------
# Fly's default on-fail restart policy retries and then leaves the machine
# `stopped` — which is indistinguishable via the Machines API from a healthy
# sleeping node. The API cannot tell those apart; this file can. Whoever owns
# the hub-side registry should read it (over the tailnet, or `fly ssh console`)
# rather than inferring "asleep and fine" from `stopped`.
EXIT_REASON="unknown"
record_exit() {
  local code="$1"
  mkdir -p "${WKS_DATA}/state" 2>/dev/null || true
  printf '{"bootId":"%s","reason":"%s","exitCode":%s,"at":"%s","machine":"%s"}\n' \
    "$BOOT_ID" "$EXIT_REASON" "$code" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${FLY_MACHINE_ID:-local}" \
    >"${WKS_DATA}/state/last-exit.json" 2>/dev/null || true
}

CLAUDEMON_PID=""
BRAIN_PID=""
DOORBELL_PID=""
TAILSCALED_PID=""

shutdown() {
  local sig="$1"
  EXIT_REASON="signal-${sig}"
  log "received SIG${sig} — this is the expected shape of a hub-driven sleep. Draining."
  # claudemon's own shutdown path kills its PTYs; its SQLite is WAL with
  # synchronous=NORMAL and written continuously, so sessions rehydrate as
  # Stopped/resumable. There is no drain handshake in the codebase yet.
  for p in "$BRAIN_PID" "$CLAUDEMON_PID" "$DOORBELL_PID" "$TAILSCALED_PID"; do
    [ -n "$p" ] && kill -TERM "$p" 2>/dev/null || true
  done
  # kill_timeout in fly.toml is 60s. Give the children 45 and keep 15 in hand.
  local waited=0
  while [ "$waited" -lt 45 ]; do
    if ! kill -0 "${CLAUDEMON_PID:-0}" 2>/dev/null && ! kill -0 "${BRAIN_PID:-0}" 2>/dev/null; then
      break
    fi
    sleep 1
    waited=$((waited + 1))
  done
  log "drained after ${waited}s"
  record_exit 0
  exit 0
}
trap 'shutdown INT' INT
trap 'shutdown TERM' TERM

die() {
  EXIT_REASON="${EXIT_REASON/#unknown/boot-failure}"
  log "FATAL: $*"
  record_exit 1
  exit 1
}

# --------------------------------------------------------------------------
# 2. Volume layout.
# --------------------------------------------------------------------------
log "bootstrapping ${WKS_DATA}"
/usr/local/lib/wks/bootstrap.sh || die "bootstrap failed — refusing to start with an unprepared volume"

# The generated half of the shell environment (bootstrap.sh seeds the .bashrc
# hook that sources this). Rewritten every boot so `fly ssh console` and any
# agent shell see exactly what the daemons see.
cat >"${WKS_HOME}/.wks-env" <<EOF
# GENERATED by entrypoint.sh on every boot. Do not edit; edit the image.
export HOME='${WKS_HOME}'
export XDG_CONFIG_HOME='${XDG_CONFIG_HOME}'
export XDG_DATA_HOME='${XDG_DATA_HOME}'
export XDG_STATE_HOME='${XDG_STATE_HOME}'
export XDG_CACHE_HOME='${XDG_CACHE_HOME}'
export GOPATH='${GOPATH}'
export GOMODCACHE='${GOMODCACHE}'
export GOCACHE='${GOCACHE}'
export BUNDLE_PATH='${BUNDLE_PATH}'
export BUN_INSTALL_CACHE_DIR='${BUN_INSTALL_CACHE_DIR}'
export npm_config_cache='${npm_config_cache}'
export HISTFILE='${HISTFILE}'
export PATH='/usr/local/go/bin:${GOPATH}/bin:/usr/local/bin:/usr/bin:/bin'
export WKS_STATE_DB='${WKS_STATE_DB}'
EOF
chown "${WKS_UID}:${WKS_GID}" "${WKS_HOME}/.wks-env" 2>/dev/null || true

# Run an unprivileged child as the wks user. Not root, deliberately: Claude
# Code refuses --dangerously-skip-permissions when euid is 0, and this fleet
# leans on full-access grants. setpriv is util-linux, already in the image.
as_wks() {
  setpriv --reuid="$WKS_UID" --regid="$WKS_GID" --init-groups --inh-caps=-all -- "$@"
}

# --------------------------------------------------------------------------
# 3. Doorbell.
# --------------------------------------------------------------------------
# fly.toml sets auto_start_machines = true / auto_stop_machines = "off": the
# Fly proxy may WAKE this machine, only the hub may sleep it. Those are two
# independent keys and the combination is legal (verified against flyctl's own
# config schema). It is a backstop for the day the hub's Fly token expires or
# the hub is down — no token is involved in this path.
#
# The proxy holds the inbound request while the machine boots and needs the
# internal port to accept a connection, so bind it EARLY and answer trivially.
# Answering here means "the hardware is up", NOT "the node is ready" — the
# node is ready when the brain registers with the hub.
#
# READ THIS BEFORE EXPOSING IT: the proxy starts the machine BEFORE routing the
# request, so no application-level auth here can stop a stranger from spending
# your money. The only real control is whether the app has a public IP at all.
# The runbook provisions with NO public IP for exactly that reason.
if [ "$DOORBELL_ENABLED" = "1" ]; then
  mkdir -p /srv/doorbell
  printf 'workspacer node awake\nboot=%s\nmachine=%s\nregion=%s\n' \
    "$BOOT_ID" "${FLY_MACHINE_ID:-local}" "${FLY_REGION:-local}" >/srv/doorbell/index.html
  cp /srv/doorbell/index.html /srv/doorbell/health
  busybox httpd -f -p "0.0.0.0:${DOORBELL_PORT}" -h /srv/doorbell &
  DOORBELL_PID=$!
  log "doorbell listening on :${DOORBELL_PORT} (pid $DOORBELL_PID) — wake backstop only"
else
  log "doorbell disabled (WKS_DOORBELL_ENABLED=0); the Machines API is then the only wake path"
fi

# --------------------------------------------------------------------------
# 4. Tailscale, in kernel mode.
# --------------------------------------------------------------------------
# Fly Machines are Firecracker microVMs with a real kernel, so this is normal
# kernel networking with a real tailscale0 interface: no --tun=userspace-
# networking, no SOCKS5, no ALL_PROXY. The image is Debian (not Alpine)
# specifically so the nftables/iptables backend tailscale needs in kernel mode
# is present and ordinary — the one way this fails confusingly on Fly.
mkdir -p /var/run/tailscale "$(dirname "$TS_STATE")"
if [ ! -c /dev/net/tun ]; then
  log "/dev/net/tun missing — creating it"
  mkdir -p /dev/net
  mknod /dev/net/tun c 10 200 2>/dev/null || log "WARNING: could not create /dev/net/tun; kernel-mode tailscaled will fail"
  chmod 600 /dev/net/tun 2>/dev/null || true
fi

log "starting tailscaled (state=${TS_STATE})"
tailscaled \
  --state="$TS_STATE" \
  --statedir="${WKS_DATA}/tailscale" \
  --socket="$TS_SOCKET" \
  --tun=tailscale0 \
  --port=41641 &
TAILSCALED_PID=$!

# Wait for the daemon's socket before talking to it.
for _ in $(seq 1 50); do
  tailscale --socket="$TS_SOCKET" status --json >/dev/null 2>&1 && break
  sleep 0.2
done

ts_backend_state() {
  tailscale --socket="$TS_SOCKET" status --json 2>/dev/null |
    sed -n 's/.*"BackendState"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1
}

state="$(ts_backend_state)"
log "tailscale backend state: ${state:-<unknown>}"

if [ "$state" = "Running" ]; then
  # Already authenticated from the persisted state file. This is the normal
  # wake path and it must NOT re-present the auth key: auth keys expire at 90
  # days maximum, and a key that has since expired would turn a healthy wake
  # into a failure.
  log "already authenticated from persisted state — reusing the existing node identity"
else
  if [ -z "${TAILSCALE_AUTHKEY:-}" ]; then
    die "tailscale needs authentication (state=${state:-unknown}) but TAILSCALE_AUTHKEY is unset. Set it with: fly secrets set TAILSCALE_AUTHKEY=tskey-auth-…"
  fi
  log "authenticating to the tailnet as '${TS_HOSTNAME}' (first boot, or the node key expired)"
  # A NON-EPHEMERAL, reusable, pre-authorized key. Tailscale's own Fly guide
  # recommends an ephemeral key; that is wrong here. An ephemeral node is
  # REMOVED from the tailnet when it goes offline, and a node that rejoins
  # after removal GETS A NEW IP — this machine would churn identity on every
  # single wake, which is the one thing the persisted state file exists to
  # prevent.
  #
  # The key should carry the tag (--advertise-tags below is belt and braces).
  # Tagging is load-bearing, not cosmetic: node keys expire after 180 days by
  # default and a SLEEPING MACHINE CANNOT REAUTHENTICATE ITSELF, but a device
  # tagged at first authentication has key expiry disabled by default.
  ts_up_args=(--authkey="$TAILSCALE_AUTHKEY" --hostname="$TS_HOSTNAME" --accept-routes=false)
  [ -n "${WKS_TS_TAGS:-}" ] && ts_up_args+=(--advertise-tags="$WKS_TS_TAGS")
  tailscale --socket="$TS_SOCKET" up "${ts_up_args[@]}" ||
    die "tailscale up failed — check the auth key is non-ephemeral, reusable, pre-authorized and unexpired"
fi

# --------------------------------------------------------------------------
# 5. Wait for the tailnet.
# --------------------------------------------------------------------------
# The brain must not dial the hub before this. Nobody publishes a tailscaled
# cold-reconnect figure, so the wait is measured here and written to the boot
# log — that number is the one unknown in the wake budget.
ts_wait_start=$(date +%s)
ts_ip=""
while [ $(($(date +%s) - ts_wait_start)) -lt "$TS_WAIT_SECS" ]; do
  ts_ip="$(tailscale --socket="$TS_SOCKET" ip -4 2>/dev/null | head -1 || true)"
  [ -n "$ts_ip" ] && break
  sleep 0.5
done
[ -n "$ts_ip" ] || die "no tailnet address after ${TS_WAIT_SECS}s — the node is unreachable, refusing to pretend otherwise"
log "TAILNET UP after $(($(date +%s) - ts_wait_start))s — ipv4=${ts_ip}"
log "  (this address must be IDENTICAL across a stop/start cycle. If it changed, the state file was not persisted.)"
tailscale --socket="$TS_SOCKET" status --peers=false 2>/dev/null | sed 's/^/    /' || true

# --------------------------------------------------------------------------
# 6. claudemon init — this entrypoint drives claudemon directly, not `serve`.
# --------------------------------------------------------------------------
# As of 9b061244, `workspacer serve` runs `claudemon init` itself (and pins
# --db-path). This entrypoint does NOT go through `serve` — it drives
# `claudemon` and `brain` directly (step 7 below) — so it still needs this
# explicit call, and it stays correct as written. It would only become
# redundant if this entrypoint switched to shelling out to `workspacer serve`
# instead.
# On a fresh volume ~/.claude/settings.json does not exist, so without this the
# hook forwarder and statusLine forwarder are absent. The symptom is NOT idle
# sessions — the opposite: internal/quiescence treats mode: "unknown" as a
# blocker, so a hookless session fails safe and pins the machine awake. A PTY
# session never leaves SessionMode::Unknown, and a spawn's first_message is
# held until the Input transition, so a dispatched PTY worker never receives
# its prompt — it just sits there looking alive and doing nothing. Idempotent:
# prints "already up to date" and writes nothing when the merge is a no-op.
log "running claudemon init (hook port ${CLAUDEMON_HOOK_PORT})"
as_wks env HOME="$WKS_HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" \
  claudemon init --hook-port "$CLAUDEMON_HOOK_PORT" ||
  log "WARNING: claudemon init failed — PTY sessions on this node may pin the machine awake without ever receiving a prompt"

# --------------------------------------------------------------------------
# 7. claudemon, then brain.
# --------------------------------------------------------------------------
log "starting claudemon (api=${CLAUDEMON_API_PORT} hook=${CLAUDEMON_HOOK_PORT} db=${WKS_STATE_DB})"
as_wks env HOME="$WKS_HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" \
  XDG_STATE_HOME="$XDG_STATE_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" PATH="$PATH" \
  GOPATH="$GOPATH" GOMODCACHE="$GOMODCACHE" GOCACHE="$GOCACHE" BUNDLE_PATH="$BUNDLE_PATH" \
  BUN_INSTALL_CACHE_DIR="$BUN_INSTALL_CACHE_DIR" npm_config_cache="$npm_config_cache" \
  claudemon serve \
  --host 127.0.0.1 \
  --api-port "$CLAUDEMON_API_PORT" \
  --hook-port "$CLAUDEMON_HOOK_PORT" \
  --db-path "$WKS_STATE_DB" &
CLAUDEMON_PID=$!

for _ in $(seq 1 100); do
  curl -sf "${CLAUDEMON_URL}/sessions" >/dev/null 2>&1 && break
  kill -0 "$CLAUDEMON_PID" 2>/dev/null || die "claudemon exited during startup — see above"
  sleep 0.2
done
curl -sf "${CLAUDEMON_URL}/sessions" >/dev/null 2>&1 || die "claudemon did not answer on ${CLAUDEMON_URL} within 20s"
log "claudemon ready (pid $CLAUDEMON_PID)"

[ -n "${HUB_BUS_URL:-}" ] || die "HUB_BUS_URL is unset — the node has nothing to attach to. fly secrets set HUB_BUS_URL=wss://<hub>/bus"
[ -n "${HUB_TOKEN:-}" ] || log "WARNING: HUB_TOKEN is unset — the brain will attach with no auth, which the hub will refuse if it requires a token"

log "starting brain, attaching to ${HUB_BUS_URL} as a capability provider"
as_wks env HOME="$WKS_HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" \
  XDG_STATE_HOME="$XDG_STATE_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" PATH="$PATH" \
  GOPATH="$GOPATH" GOMODCACHE="$GOMODCACHE" GOCACHE="$GOCACHE" BUNDLE_PATH="$BUNDLE_PATH" \
  BUN_INSTALL_CACHE_DIR="$BUN_INSTALL_CACHE_DIR" npm_config_cache="$npm_config_cache" \
  brain \
  --hub "$HUB_BUS_URL" \
  --token "${HUB_TOKEN:-}" \
  --claudemon "$CLAUDEMON_URL" \
  --scope "${WKS_BRAIN_SCOPE:-full}" &
BRAIN_PID=$!
log "brain started (pid $BRAIN_PID). The node is READY once the hub logs the provider registration."
log "BOOT COMPLETE $BOOT_ID"

# --------------------------------------------------------------------------
# Supervise. Either child dying is fatal: Fly's restart policy is on-failure
# with a small retry count, so a genuine crash restarts once or twice and then
# stops rather than thrashing. The reason is on the volume either way.
# --------------------------------------------------------------------------
while true; do
  if ! kill -0 "$CLAUDEMON_PID" 2>/dev/null; then
    wait "$CLAUDEMON_PID" 2>/dev/null || true
    EXIT_REASON="claudemon-died"
    log "claudemon exited unexpectedly — bringing the node down so the hub sees a clean disconnect"
    kill -TERM "$BRAIN_PID" 2>/dev/null || true
    record_exit 1
    exit 1
  fi
  if ! kill -0 "$BRAIN_PID" 2>/dev/null; then
    wait "$BRAIN_PID" 2>/dev/null || true
    EXIT_REASON="brain-died"
    log "brain exited unexpectedly — the node has no provider, bringing it down"
    kill -TERM "$CLAUDEMON_PID" 2>/dev/null || true
    record_exit 1
    exit 1
  fi
  sleep 5
done
