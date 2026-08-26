#!/usr/bin/env bash
#
# entrypoint.sh — PID 1 on the ALWAYS-ON Fly hub.
#
# This machine exists because the worker node sleeps, and something has to be
# awake to receive "wake node X" from a phone, hold the Fly API token, and run
# the node registry. That cannot be the node (a sleeping hub cannot wake
# anything) and it should not be a desktop (then waking from a phone only works
# while the desktop is on).
#
# It runs exactly ONE workspacer process:
#
#   hub --brain-scope off --addr 127.0.0.1:7895 …
#
# No claudemon. No brain. No Claude Code sessions. Nothing that holds an agent
# credential. The machine that is always on and holds a token which spends money
# is deliberately the machine with the least on it.
#
#   *** --brain-scope off IS REQUIRED, NOT PREFERRED. ***
#
# The node registry's liveness probe is brain.info, and it cannot tell a LOCAL
# brain from a REMOTE one. A hub supervising its own brain would report a stopped
# node as `available`, forever, and every wake would appear to have already
# happened. cmd/hub/nodes.go logs a warning about exactly this combination and
# does not enforce it; these artifacts do. It is also the flag's own default —
# it is passed explicitly anyway, because a default is not a decision.
#
# Boot order and why:
#   1. boot log on the volume       a crash loop at 3am must be readable at 9am;
#                                   Fly retains logs 7 days
#   2. bootstrap the volume         refuses an unmounted volume, and decides
#                                   FIRST RUN vs STATE LOSS per file. This is the
#                                   step that stops a hub silently becoming a
#                                   DIFFERENT hub. It may exit 2; that is correct.
#   3. tailscaled + tailscale up    the tailnet is the hub's only network surface
#   4. wait for the tailnet         the hub binds loopback and is reached ONLY
#                                   through `tailscale serve`, so no tailnet
#                                   means no hub. Refuse rather than pretend.
#   5. tailscale serve              terminates TLS with a real Let's Encrypt cert
#                                   for the MagicDNS name. Load-bearing: /m is a
#                                   PWA and service workers + Web Push REQUIRE a
#                                   secure context.
#   6. hub                          with --trusted-host set to the name step 5
#                                   just proved, derived rather than configured
#   6b. one end-to-end HTTPS request  serve returning 0 is not the hub being
#                                      reachable; the certificate is fetched on
#                                      demand and can fail afterwards. NON-FATAL.
#   7. supervise + a loopback health watchdog
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
# Pinned under $HOME rather than at a separate /data/config. One variable moves
# five state locations: authtoken.ConfigDir() reads it directly (remote-token,
# tokens.json, peers.json, nodes.json), and Go's os.UserConfigDir() — which is
# what defaultPushDir, defaultJobsFile and defaultLayoutFile use — reads it too.
# Pinning it somewhere other than $HOME/.config would split the Go half from the
# TS half of config.yaml, which hardcodes ~/.config/workspacer.
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$WKS_HOME/.config}"
export XDG_DATA_HOME="${XDG_DATA_HOME:-$WKS_HOME/.local/share}"
export XDG_STATE_HOME="${XDG_STATE_HOME:-$WKS_HOME/.local/state}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-$WKS_DATA/cache/xdg}"
export HISTFILE="${HISTFILE:-$WKS_HOME/.bash_history}"

HUB_BIND="${WKS_HUB_BIND:-127.0.0.1}"
HUB_PORT="${WKS_HUB_PORT:-7895}"
HUB_HEALTH="http://127.0.0.1:${HUB_PORT}/health"
WEBAPP_DIR="${WKS_HUB_WEBAPP_DIR:-/usr/local/share/workspacer/web}"
PLUGINS_DIR="${WKS_HUB_PLUGINS_DIR:-$XDG_CONFIG_HOME/workspacer/plugins}"

TS_STATE="${WKS_DATA}/tailscale/tailscaled.state"
TS_SOCKET="${TS_SOCKET:-/var/run/tailscale/tailscaled.sock}"
TS_HOSTNAME="${WKS_TS_HOSTNAME:-${FLY_APP_NAME:-workspacer-hub}}"
TS_WAIT_SECS="${WKS_TAILNET_WAIT_SECS:-60}"

SERVE_ENABLED="${WKS_HUB_SERVE_ENABLED:-1}"
SERVE_PORT="${WKS_HUB_SERVE_PORT:-443}"
# A SECOND origin that also routes here, so a browser at /app can frame a
# hub-served plugin cross-origin and that plugin keeps its bus link. Same-origin,
# the browser must sandbox it opaque (it could otherwise read the app's host
# token), and the plugin paints but can talk to nothing. Tailscale allows serve
# on 443, 8443 and 10000; 8443 is the second of those.
PLUGIN_ORIGIN_ENABLED="${WKS_HUB_PLUGIN_ORIGIN_ENABLED:-1}"
PLUGIN_ORIGIN_PORT="${WKS_HUB_PLUGIN_ORIGIN_PORT:-8443}"

# The end-to-end reachability probe. See step 6b, non-fatal, by design and by
# construction. Set WKS_HUB_SERVE_PROBE_ENABLED=0 to skip it entirely.
SERVE_PROBE_ENABLED="${WKS_HUB_SERVE_PROBE_ENABLED:-1}"
SERVE_PROBE_SECS="${WKS_HUB_SERVE_PROBE_SECS:-30}"

HEALTH_INTERVAL="${WKS_HUB_HEALTH_INTERVAL:-30}"
HEALTH_FAILURES="${WKS_HUB_HEALTH_FAILURES:-4}"

BOOT_LOG="${WKS_DATA}/logs/boot.log"
LAST_BOOT_LOG="${WKS_DATA}/logs/last-boot.log"
BOOT_LOG_MAX_BYTES="${WKS_BOOT_LOG_MAX_BYTES:-5242880}" # 5 MiB
BOOT_ID="$(date -u +%Y%m%dT%H%M%SZ)-${FLY_MACHINE_ID:-local}"

# --------------------------------------------------------------------------
# 1. The boot log, before anything else runs.
# --------------------------------------------------------------------------
# Fly retains logs for 7 days. This machine is always on, so unlike the node it
# is usually there to be asked — but the failure this deployment is built around
# is a RESTART that loses state, and a restart loop that started nine days ago
# with one interesting line at the top is exactly the case Fly's retention drops.
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
# CONSUMED, not just read. See the node entrypoint for the full argument; it
# applies here unchanged. The short form: this file is written by a trap, a
# SIGKILL runs no trap, and a record left in place is silently re-reported as
# though it described the run that just died. Renaming it after logging means the
# answer for a run that was killed without warning is "no record", which is true,
# rather than the previous run's reason, which is not.
if [ -f "${WKS_DATA}/state/last-exit.json" ]; then
  log "  previous run ended: $(cat "${WKS_DATA}/state/last-exit.json")"
  mv -f "${WKS_DATA}/state/last-exit.json" "${WKS_DATA}/state/last-exit.consumed.json" 2>/dev/null || true
else
  log "  previous run ended: <no record. Either a first boot, a volume that was not mounted, or a run"
  log "    that was KILLED WITHOUT WARNING (host eviction, OOM, SIGKILL): no trap runs, so nothing is written>"
fi

# --------------------------------------------------------------------------
# Record why this run ended.
# --------------------------------------------------------------------------
# The hub's restart policy is `always` and it is never legitimately stopped, so
# unlike the node there is no "asleep vs crashed" ambiguity to resolve. This file
# answers a different question: a machine that has restarted six times in an hour
# looks identical to one that has been up all day, and the reason for each
# restart is the thing you actually need.
EXIT_REASON="unknown"
record_exit() {
  local code="$1"
  mkdir -p "${WKS_DATA}/state" 2>/dev/null || true
  printf '{"bootId":"%s","reason":"%s","exitCode":%s,"at":"%s","machine":"%s"}\n' \
    "$BOOT_ID" "$EXIT_REASON" "$code" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "${FLY_MACHINE_ID:-local}" \
    >"${WKS_DATA}/state/last-exit.json" 2>/dev/null || true
}

HUB_PID=""
TAILSCALED_PID=""

shutdown() {
  local sig="$1"
  EXIT_REASON="signal-${sig}"
  log "received SIG${sig} — draining."
  # The hub's own SIGTERM path tears down plugin sidecars and closes bus
  # connections, so every client sees a clean disconnect rather than a hang. Its
  # persistent state (jobs, layout, push subscriptions) is written on change, not
  # on exit, so there is nothing to flush.
  for p in "$HUB_PID" "$TAILSCALED_PID"; do
    [ -n "$p" ] && kill -TERM "$p" 2>/dev/null || true
  done
  local waited=0
  while [ "$waited" -lt 20 ]; do
    kill -0 "${HUB_PID:-0}" 2>/dev/null || break
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
# 2. Volume layout, and the state guard.
# --------------------------------------------------------------------------
# This is the step that separates FIRST RUN from STATE LOSS. It exits 2 when a
# create-once file has vanished from a volume that has held it before — because
# a hub that starts on lost state does not fail, it silently becomes a different
# hub. Do not "fix" a bootstrap exit 2 by removing this check.
log "bootstrapping ${WKS_DATA}"
# `bootstrap_rc=0; cmd || bootstrap_rc=$?` rather than `if ! cmd; then rc=$?`.
# In the latter, $? inside the branch is the status of the NEGATED pipeline —
# always 0 — so the exit code is lost and the `= 2` test below can never fire.
# A genuine state-loss refusal would then be reported as a generic bootstrap
# failure with "(exit 0)" in the message, which is exactly the kind of misleading
# signal this file exists to prevent. Caught by running the built image.
bootstrap_rc=0
/usr/local/lib/wks-hub/bootstrap.sh || bootstrap_rc=$?
if [ "$bootstrap_rc" -ne 0 ]; then
  rc="$bootstrap_rc"
  if [ "$rc" = 2 ]; then
    EXIT_REASON="state-loss"
    log "FATAL: bootstrap refused to prepare the volume — see the STATE LOSS lines above."
    log "  This machine is deliberately crash-looping instead of coming up as somebody else."
    record_exit 2
    exit 2
  fi
  die "bootstrap failed (exit $rc) — refusing to start with an unprepared volume"
fi

# The generated half of the shell environment (bootstrap seeds the .bashrc hook
# that sources this). Rewritten every boot so `fly ssh console` sees exactly what
# the hub sees.
cat >"${WKS_HOME}/.wks-env" <<EOF
# GENERATED by entrypoint.sh on every boot. Do not edit; edit the image.
export HOME='${WKS_HOME}'
export XDG_CONFIG_HOME='${XDG_CONFIG_HOME}'
export XDG_DATA_HOME='${XDG_DATA_HOME}'
export XDG_STATE_HOME='${XDG_STATE_HOME}'
export XDG_CACHE_HOME='${XDG_CACHE_HOME}'
export HISTFILE='${HISTFILE}'
export PATH='/usr/local/bin:/usr/bin:/bin'
EOF
chown "${WKS_UID}:${WKS_GID}" "${WKS_HOME}/.wks-env" 2>/dev/null || true

# Run an unprivileged child as the wks user. The entrypoint itself must be root
# (tailscaled, and the volume chown), and drops privilege for the hub: every
# state file on the volume is 0600 owned by 10001, and an always-on daemon that
# is reachable by anything at all has no business being root.
#
# `--inh-caps`, not `--inherit-caps`. util-linux 2.41.5 (Debian trixie, which is
# what the base image ships) has --inh-caps, --ambient-caps and --bounding-set,
# and no --inherit-caps at all. setpriv rejects the unknown option and exits
# BEFORE running anything, so the mistake does not degrade privilege dropping —
# it silently starts nothing, and surfaces one step later as "the hub exited
# during startup", pointing at the wrong file.
as_wks() {
  setpriv --reuid="$WKS_UID" --regid="$WKS_GID" --init-groups --inh-caps=-all -- "$@"
}

# Prove it before relying on it. This costs a fork and closes the whole class:
# any future change to setpriv's option set, to the image's util-linux, or to
# the wks user fails HERE, naming the mechanism, instead of three steps later
# as an unexplained daemon that would not start.
as_wks true 2>/tmp/as_wks.err || die "cannot drop privilege to ${WKS_USER} (${WKS_UID}:${WKS_GID}): $(cat /tmp/as_wks.err 2>/dev/null). The hub must not run as root."
rm -f /tmp/as_wks.err

# --------------------------------------------------------------------------
# 3. Tailscale, in kernel mode.
# --------------------------------------------------------------------------
# Fly Machines are Firecracker microVMs with a real kernel, so this is ordinary
# kernel networking with a real tailscale0 interface: no --tun=userspace-
# networking, no SOCKS5, no ALL_PROXY. The base image is Debian (not Alpine)
# specifically so the nftables/iptables backend kernel mode needs is present and
# ordinary — the one way this fails confusingly on Fly.
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

for _ in $(seq 1 50); do
  tailscale --socket="$TS_SOCKET" status --json >/dev/null 2>&1 && break
  sleep 0.2
done

ts_status_field() {
  tailscale --socket="$TS_SOCKET" status --json 2>/dev/null |
    tr ',' '\n' | sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -1
}

state="$(ts_status_field BackendState)"
log "tailscale backend state: ${state:-<unknown>}"

if [ "$state" = "Running" ]; then
  # Already authenticated from the persisted state file. Must NOT re-present the
  # auth key: auth keys expire at 90 days maximum, and a key that has since
  # expired would turn a healthy restart into a failure.
  log "already authenticated from persisted state — reusing the existing hub identity"
else
  if [ -z "${TAILSCALE_AUTHKEY:-}" ]; then
    die "tailscale needs authentication (state=${state:-unknown}) but TAILSCALE_AUTHKEY is unset. Set it with: fly secrets set TAILSCALE_AUTHKEY=tskey-auth-…"
  fi
  log "authenticating to the tailnet as '${TS_HOSTNAME}' (first boot, or the node key expired)"
  # NON-EPHEMERAL, reusable, pre-authorized, TAGGED. Tailscale's own Fly guide
  # recommends an ephemeral key; that is wrong for both machines in this design
  # and worse for this one — an ephemeral node is REMOVED from the tailnet when
  # it goes offline and gets a NEW address when it rejoins, and this is the
  # address the sleeping node's HUB_BUS_URL names. A restart would strand the
  # node with no way back.
  #
  # Tagging is load-bearing, not cosmetic: node keys expire after 180 days by
  # default, but a device TAGGED at first authentication has key expiry disabled
  # by default. Without the tag this works for six months and then stops, with
  # no signal anyone would connect to the cause.
  ts_up_args=(--authkey="$TAILSCALE_AUTHKEY" --hostname="$TS_HOSTNAME" --accept-routes=false)
  [ -n "${WKS_TS_TAGS:-}" ] && ts_up_args+=(--advertise-tags="$WKS_TS_TAGS")
  tailscale --socket="$TS_SOCKET" up "${ts_up_args[@]}" ||
    die "tailscale up failed — check the auth key is non-ephemeral, reusable, pre-authorized and unexpired"
fi

# --------------------------------------------------------------------------
# 4. Wait for the tailnet.
# --------------------------------------------------------------------------
# The hub binds loopback and is reached only through `tailscale serve`, so no
# tailnet means no hub — not a degraded hub. Refuse rather than come up serving
# nothing on an interface nobody can reach.
ts_wait_start=$(date +%s)
ts_ip=""
while [ $(($(date +%s) - ts_wait_start)) -lt "$TS_WAIT_SECS" ]; do
  ts_ip="$(tailscale --socket="$TS_SOCKET" ip -4 2>/dev/null | head -1 || true)"
  [ -n "$ts_ip" ] && break
  sleep 0.5
done
[ -n "$ts_ip" ] || die "no tailnet address after ${TS_WAIT_SECS}s — the hub would be unreachable, refusing to pretend otherwise"
log "TAILNET UP after $(($(date +%s) - ts_wait_start))s — ipv4=${ts_ip}"
log "  (this address must be IDENTICAL across a restart. If it changed, tailscaled.state was not persisted"
log "   and the node's HUB_BUS_URL now names a different device.)"

# The MagicDNS name, DERIVED rather than configured. It is what `tailscale serve`
# gets its certificate for, what the node's HUB_BUS_URL must name, and what the
# hub must be told to trust — three places that cannot be allowed to disagree, so
# none of them is a hand-typed value.
TS_DNSNAME="$(ts_status_field DNSName | sed 's/\.$//')"
if [ -z "$TS_DNSNAME" ]; then
  log "WARNING: could not read this device's MagicDNS name from tailscale status."
  log "  Falling back to the tailnet IP. TLS via \`tailscale serve\` needs the name, so it will likely fail."
fi
log "MagicDNS name: ${TS_DNSNAME:-<unknown>}"

# --------------------------------------------------------------------------
# 5. tailscale serve — TLS, and the only way in.
# --------------------------------------------------------------------------
# THIS IS NOT A CONVENIENCE. /m is an installable PWA whose entire value on this
# deployment is background Web Push ("an agent needs you"), and both
# navigator.serviceWorker and PushManager REQUIRE a secure context. Plain HTTP on
# a tailnet IP is not one. `tailscale serve` terminates TLS with a real Let's
# Encrypt certificate for the MagicDNS name, on a machine with no public IP —
# which is the combination this design needs and the reason there is no
# [http_service] block in fly.toml.
#
# It proxies to 127.0.0.1, which means the hub's own socket lands on loopback
# while the Host header carries a public-looking name. That is byte-for-byte the
# DNS-rebinding shape the hub's Host/Origin pins refuse, so --trusted-host below
# is REQUIRED — without it every route behind the proxy answers 403, including
# /health, which reads as "the hub is broken".
TRUSTED_HOSTS="${WKS_HUB_TRUSTED_HOSTS:-}"
if [ -n "$TS_DNSNAME" ]; then
  TRUSTED_HOSTS="${TS_DNSNAME}${TRUSTED_HOSTS:+,$TRUSTED_HOSTS}"
fi

PLUGIN_ORIGIN=""
if [ "$SERVE_ENABLED" = "1" ]; then
  # Reset first: a rule left by an older image would otherwise linger in
  # tailscaled's persisted state and quietly shadow the one below.
  tailscale --socket="$TS_SOCKET" serve reset >/dev/null 2>&1 || true

  log "tailscale serve --bg --https=${SERVE_PORT} → http://127.0.0.1:${HUB_PORT}"
  tailscale --socket="$TS_SOCKET" serve --bg --https="$SERVE_PORT" "http://127.0.0.1:${HUB_PORT}" || die \
    "tailscale serve failed. The three things that cause this, in order of likelihood:
    1. HTTPS certificates are not enabled for your tailnet (admin console → DNS → HTTPS Certificates).
    2. This device is tagged and the tag is not permitted to fetch certificates.
    3. Your tailscale build wants the older \`serve https:${SERVE_PORT} / <target>\` syntax.
  The hub binds loopback, so without serve there is NO way in. To fall back to plain HTTP on the
  tailnet IP — losing the PWA and Web Push, which need a secure context — set
  WKS_HUB_SERVE_ENABLED=0 and WKS_HUB_BIND=0.0.0.0."

  if [ "$PLUGIN_ORIGIN_ENABLED" = "1" ] && [ -n "$TS_DNSNAME" ]; then
    if tailscale --socket="$TS_SOCKET" serve --bg --https="$PLUGIN_ORIGIN_PORT" "http://127.0.0.1:${HUB_PORT}" 2>/dev/null; then
      PLUGIN_ORIGIN="https://${TS_DNSNAME}:${PLUGIN_ORIGIN_PORT}"
      # No second TRUSTED_HOSTS entry for the plugin origin. It was here, and it
      # was a no-op that printed the MagicDNS name twice in the startup log,
      # which reads as a misconfiguration to anyone checking their `fly logs` at
      # midnight. The hub strips the port before matching a Host header, so the
      # name added above already covers :443 and :8443 alike. Verified against
      # the running hub: `Host: <name>` and `Host: <name>:8443` both answer 200,
      # and an unrelated host still gets 403.
      log "plugin origin: ${PLUGIN_ORIGIN} (a second origin so /app can frame plugin UI cross-origin)"
    else
      log "WARNING: could not serve the second origin on :${PLUGIN_ORIGIN_PORT}. Plugin panes will still"
      log "  render in a remote browser, but same-origin framing means the browser sandboxes them opaque"
      log "  and each one loses its bus connection. Set WKS_HUB_PLUGIN_ORIGIN_ENABLED=0 to stop trying."
    fi
  fi

  tailscale --socket="$TS_SOCKET" serve status 2>/dev/null | sed 's/^/    /' || true
else
  log "tailscale serve DISABLED (WKS_HUB_SERVE_ENABLED=0) — the hub is reachable only at"
  log "  http://${ts_ip}:${HUB_PORT} over the tailnet. No TLS, so /m's service worker and Web Push"
  log "  will not work: a service worker requires a secure context."
fi

# --------------------------------------------------------------------------
# 6. The hub.
# --------------------------------------------------------------------------
HUB_TOKEN_FILE="${XDG_CONFIG_HOME}/workspacer/remote-token"
[ -s "$HUB_TOKEN_FILE" ] || die "no pairing credential at $HUB_TOKEN_FILE — bootstrap should have minted or refused. This is a bug in the entrypoint."

# THE CREDENTIAL TRAVELS IN THE ENVIRONMENT, NEVER IN ARGV. The --token flag
# defaults to $HUB_TOKEN precisely so it does not have to be typed, and
# /proc/<pid>/cmdline is world-readable — the same reasoning nodes.go gives for
# refusing to accept the Fly token as a flag. `workspacer serve` passes --token
# because it is building a child's argv from a value it already holds; here there
# is no such constraint, so the weaker shape is not used.
HUB_TOKEN="$(tr -d '[:space:]' <"$HUB_TOKEN_FILE")"
export HUB_TOKEN

# $FLY_API_TOKEN, if set, is the LAST place nodes.ResolveToken looks, after each
# entry's inline `token` and `tokenFile`. Preferring it is a deliberate security
# decision for this machine: the credential then lives in Fly's secret store
# rather than on the volume, so it is not copied by a volume snapshot, and
# rotating it is `fly secrets set` rather than editing a file inside a running
# machine. nodes.json is then pure topology and not a credential file at all.
if [ -f "${XDG_CONFIG_HOME}/workspacer/nodes.json" ]; then
  if [ -z "${FLY_API_TOKEN:-}" ] && ! grep -q '"token"\|"tokenFile"' "${XDG_CONFIG_HOME}/workspacer/nodes.json" 2>/dev/null; then
    log "WARNING: a node registry exists but no Fly credential is resolvable — \$FLY_API_TOKEN is unset and"
    log "  no entry carries \`token\` or \`tokenFile\`. Nodes will be REPORTED but cannot be WOKEN from here,"
    log "  which is this machine's only job. Fix: fly secrets set FLY_API_TOKEN='FlyV1 fm2_…'"
  fi
else
  log "no node registry at ${XDG_CONFIG_HOME}/workspacer/nodes.json — nodes.list and nodes.wake will not be"
  log "  registered and no node can be woken. See RUNBOOK.md §8."
fi

hub_args=(
  --addr "${HUB_BIND}:${HUB_PORT}"
  # REQUIRED. The node registry probes liveness with brain.info, which cannot
  # tell a local brain from a remote one, so a hub that supervises its own brain
  # reports a stopped node as `available` forever. See the header.
  --brain-scope off
  --plugins-dir "$PLUGINS_DIR"

  # *** JOBS OFF, AND THIS IS A SECURITY BOUNDARY, NOT A PREFERENCE. ***
  #
  # Follow the authority through. The node attaches with an OPERATOR-tier
  # $HUB_TOKEN. Operator tier is `trusted`. Every jobs.* method is gated by
  # jobsTrusted (cmd/hub/main.go), and jobsTrusted is a bare c.IsTrusted(),
  # nothing narrower. So the node may call jobs.upsert and then jobs.run, and a
  # job of kind shell reaches jobs.BusRunner.Shell, which is
  # exec.CommandContext("/bin/sh", "-c", command). Unconfined.
  #
  # Where does that /bin/sh run? Not on the node. In THIS process's environment,
  # on THIS machine: the one holding $FLY_API_TOKEN, on the volume holding
  # nodes.json, tokens.json and remote-token. And the node is, by design, the
  # machine that runs code an agent wrote. That is a straight path from a
  # prompt-injected agent to the credential that creates and destroys machines
  # and spends the money.
  #
  # Nothing on this hub schedules a job: it runs `hub` and nothing else, has no
  # brain, and no plugin ships here, so the whole subsystem costs nothing to
  # switch off, and switching it off removes the reachable code rather than
  # arguing about the gate in front of it. An EMPTY --jobs-file is the documented
  # off switch: `jobsFile` defaults to defaultJobsFile(), and `if *jobsFile != ""`
  # is what wraps every jobs.* RegisterLocalIdent, so an empty value registers
  # none of them and starts no scheduler. Verified in cmd/hub/main.go at the flag
  # declaration and at the guard, and asserted executably by preflight.sh, which
  # runs `workspacer jobs list` against a booted hub and requires it to fail.
  #
  # The day this hub needs a job, the fix is NOT to delete this line. It is to
  # give the node a narrower tier than operator, or jobs.* a gate narrower than
  # IsTrusted.
  --jobs-file ""
)
if [ -n "$TRUSTED_HOSTS" ]; then
  hub_args+=(--trusted-host "$TRUSTED_HOSTS")
fi
if [ -n "$PLUGIN_ORIGIN" ]; then
  hub_args+=(--plugin-origin "$PLUGIN_ORIGIN")
fi
if [ -d "$WEBAPP_DIR" ] && [ -f "$WEBAPP_DIR/index.html" ]; then
  hub_args+=(--webapp-dir "$WEBAPP_DIR")
  log "serving the full web app at /app/ from $WEBAPP_DIR"
else
  log "no web app at $WEBAPP_DIR — /app will be disabled. /m (the PWA) is compiled into the binary and"
  log "  is unaffected. Rebuild with --build-arg WKS_WITH_WEBAPP=1 to include it."
fi

# Plugin sidecars run confined. `enforce` refuses to start a sidecar on a
# platform with no confinement mechanism (fail closed) — the right posture for
# the machine that holds a token which spends money. The mechanism on Linux is
# bubblewrap, installed in this image for exactly this reason. UNVERIFIED on Fly:
# bwrap needs unprivileged user namespaces, which a Firecracker guest kernel may
# or may not allow. No plugin ships here by default, so if it is wrong the first
# person to install one gets a clear refusal rather than a silent unconfined
# sidecar. Set WORKSPACER_PLUGIN_SANDBOX=best-effort to fall back.
export WORKSPACER_PLUGIN_SANDBOX="${WORKSPACER_PLUGIN_SANDBOX:-enforce}"

log "starting hub on ${HUB_BIND}:${HUB_PORT} (brain-scope off, trusted-host=${TRUSTED_HOSTS:-<none>})"
as_wks env HOME="$WKS_HOME" XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" \
  XDG_STATE_HOME="$XDG_STATE_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" \
  hub "${hub_args[@]}" &
HUB_PID=$!

for _ in $(seq 1 100); do
  curl -sf -H 'Host: 127.0.0.1' "$HUB_HEALTH" >/dev/null 2>&1 && break
  kill -0 "$HUB_PID" 2>/dev/null || die "hub exited during startup — see above"
  sleep 0.2
done
curl -sf -H 'Host: 127.0.0.1' "$HUB_HEALTH" >/dev/null 2>&1 || die "hub did not answer on ${HUB_HEALTH} within 20s"

log "hub ready (pid $HUB_PID)"

# --------------------------------------------------------------------------
# 6b. PROVE THE CHAIN A CLIENT ACTUALLY USES.
# --------------------------------------------------------------------------
# Everything green so far says nothing about whether anybody can reach this hub.
#
#   `tailscale serve --bg` INSTALLS A CONFIG. It does not fetch a certificate.
#   The Let's Encrypt certificate for the MagicDNS name is fetched ON DEMAND, on
#   the first request, and it can fail long after serve returned 0, DNS→HTTPS
#   Certificates not enabled for the tailnet, or a tagged device whose tag is not
#   permitted to fetch certs. RUNBOOK.md lists that second one as the largest
#   unverified assumption in the whole deployment.
#
#   The health watchdog below polls `-H 'Host: 127.0.0.1' http://127.0.0.1:7895`.
#   That is the loopback socket with a loopback Host header. It never traverses
#   serve, never presents the MagicDNS Host, and never touches TLS. It is the
#   right check for "is the process wedged" and it is no check at all for "can my
#   phone open this".
#
# So one request, over the whole chain: DNS for the MagicDNS name, TLS with the
# on-demand certificate, serve's proxy to loopback, and the hub's own Host pin,
# which is the fourth thing this proves, because --trusted-host is what stops
# that request being a 403 that reads as "the hub is broken".
#
# *** NON-FATAL. THIS MUST NEVER STOP A BOOT. ***
#
# Not a preference. This repo shipped an image that could not boot because an
# entrypoint line exited before running anything, and a probe that can fail a
# boot would be that mistake again with better intentions. A hub that is up but
# unreachable is strictly better than a hub that is down: the operator can still
# `fly ssh console` into the first one. So the whole block lives inside an `if`,
# every curl is `|| true`-shaped by construction, and the failure path logs a
# triage list and falls through.
if [ "$SERVE_PROBE_ENABLED" = "1" ] && [ "$SERVE_ENABLED" = "1" ] && [ -n "$TS_DNSNAME" ]; then
  probe_url="https://${TS_DNSNAME}/health"
  log "probing ${probe_url} over the whole chain, the way a client uses it"
  probe_start=$(date +%s)
  probe_ok=0
  probe_err=""
  while [ $(($(date +%s) - probe_start)) -lt "$SERVE_PROBE_SECS" ]; do
    # The first request is the one that triggers certificate issuance, so it is
    # allowed to be slow and to fail while that completes. --max-time bounds each
    # attempt; the loop bounds the total.
    if probe_err="$(curl -sS --max-time 10 -o /dev/null -w '%{http_code}' "$probe_url" 2>&1)" &&
      [ "$probe_err" = "200" ]; then
      probe_ok=1
      break
    fi
    sleep 2
  done
  probe_took=$(($(date +%s) - probe_start))
  # curl -sS puts its diagnostic on stderr and the -w code on stdout, so a
  # failure gives a two-line value. Flatten it: one log line per fact.
  probe_err="$(printf '%s' "$probe_err" | tr '\n' ' ' | sed 's/  */ /g; s/ $//')"
  if [ "$probe_ok" = 1 ]; then
    log "REACHABLE: ${probe_url} answered 200 after ${probe_took}s."
    log "  DNS, the certificate, serve's proxy and the hub's Host pin are all working."
    log "  A phone on this tailnet can open /m."
  else
    log "WARNING: ${probe_url} did NOT answer within ${probe_took}s. Last result: ${probe_err:-<none>}"
    log "  The hub itself is UP and answering on loopback: this is the boot continuing, not failing."
    log "  What this means is that the path a CLIENT uses is broken, and \`tailscale serve\` returning 0"
    log "  did not tell you so, because serve installs a config and the certificate is fetched on demand."
    log "  In order of likelihood, from a shell on this machine"
    log "  (\`fly ssh console --app ${FLY_APP_NAME:-workspacer-hub}\`):"
    log "    1. Is there a certificate?  tailscale cert ${TS_DNSNAME}"
    log "       The two causes: HTTPS Certificates are off for the tailnet (admin console → DNS), or"
    log "       this device is TAGGED and the tag is not permitted to fetch certs."
    log "    2. Is the rule installed?   tailscale serve status"
    log "       It should show :${SERVE_PORT} proxying to http://127.0.0.1:${HUB_PORT}."
    log "    3. What is the actual answer?  curl -v ${probe_url}"
    log "       A 403 is the Host pin, not the certificate. trusted-host is ${TRUSTED_HOSTS:-<none>}."
    log "    4. Does the NAME resolve, from another tailnet device? MagicDNS may be off, or the name"
    log "       may belong to a stale duplicate device, in which case THIS hub took a suffixed name,"
    log "       and the MagicDNS name logged above is the one to use everywhere."
    log "  Until this answers, the node cannot attach and no phone or browser can reach /m or /app."
  fi
elif [ "$SERVE_PROBE_ENABLED" != "1" ]; then
  log "end-to-end reachability probe disabled (WKS_HUB_SERVE_PROBE_ENABLED=0), nothing has checked that"
  log "  a client can reach this hub. The loopback watchdog below cannot: it never traverses serve."
fi

if [ -n "$TS_DNSNAME" ] && [ "$SERVE_ENABLED" = "1" ]; then
  log "  bus     wss://${TS_DNSNAME}/bus      ← the node's HUB_BUS_URL"
  log "  phone   https://${TS_DNSNAME}/m?token=…"
  log "  browser https://${TS_DNSNAME}/app/?token=…"
fi
log "BOOT COMPLETE $BOOT_ID"

# --------------------------------------------------------------------------
# 7. Supervise, and watch the health endpoint.
# --------------------------------------------------------------------------
# The health watchdog exists because of the bind. Fly's own [[http_service.checks]]
# reach a machine through the proxy, at its non-loopback 6PN address — and this
# hub binds 127.0.0.1 on purpose, so a Fly health check could never reach it and
# would report every healthy boot as dead. The compensating control is local: poll
# the same endpoint from inside, and if it stops answering while the process is
# still alive, kill it so the platform's restart policy (`always`) takes over.
# A wedged hub is worse than a dead one here — a dead one comes back.
health_fails=0
while true; do
  if ! kill -0 "$HUB_PID" 2>/dev/null; then
    wait "$HUB_PID" 2>/dev/null || true
    EXIT_REASON="hub-died"
    log "hub exited unexpectedly — bringing the machine down so Fly restarts it"
    record_exit 1
    exit 1
  fi
  if ! kill -0 "$TAILSCALED_PID" 2>/dev/null; then
    EXIT_REASON="tailscaled-died"
    log "tailscaled exited — the hub is unreachable even though it is running. Bringing the machine down."
    kill -TERM "$HUB_PID" 2>/dev/null || true
    record_exit 1
    exit 1
  fi
  if curl -sf --max-time 5 -H 'Host: 127.0.0.1' "$HUB_HEALTH" >/dev/null 2>&1; then
    if [ "$health_fails" -gt 0 ]; then
      log "health recovered after ${health_fails} consecutive failure(s)"
      health_fails=0
    fi
  else
    health_fails=$((health_fails + 1))
    log "WARNING: ${HUB_HEALTH} did not answer (${health_fails}/${HEALTH_FAILURES})"
    if [ "$health_fails" -ge "$HEALTH_FAILURES" ]; then
      EXIT_REASON="hub-wedged"
      log "hub is alive but has not answered /health ${HEALTH_FAILURES} times running — killing it so Fly restarts the machine"
      kill -TERM "$HUB_PID" 2>/dev/null || true
      sleep 5
      kill -KILL "$HUB_PID" 2>/dev/null || true
      record_exit 1
      exit 1
    fi
  fi
  sleep "$HEALTH_INTERVAL"
done
