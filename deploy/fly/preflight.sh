#!/usr/bin/env bash
#
# preflight.sh: everything you can prove about a Fly deployment WITHOUT a Fly
# account, a tailnet, a credential or a single cent.
#
# Run this before you sit down to provision. It takes tens of minutes on a cold
# cache and about a minute on a warm one, it needs nothing but docker, and when
# it is green the only things left are the three that genuinely need a human.
#
#   ./deploy/fly/preflight.sh          # everything
#   ./deploy/fly/preflight.sh build    # just the images
#   ./deploy/fly/preflight.sh boot     # just the boot rehearsal
#
# ---------------------------------------------------------------------------
# WHY THE BOOT REHEARSAL STAGE EXISTS
# ---------------------------------------------------------------------------
# This repo shipped a node image that could not boot, and every check it had
# passed over the bug. `entrypoint.sh` called `setpriv --inherit-caps=-all`,
# which util-linux does not have (it spells it `--inh-caps`). setpriv rejected
# the unknown option and exited before running anything. A full local build, a
# 63-assertion bootstrap suite, hadolint, `docker build --check`, shellcheck and
# an empty-volume smoke test were all green, because tailscaled cannot come up
# on a build host, so the entrypoint never reached that line.
#
# A build proves ASSEMBLY. It does not prove BOOT. The `boot` stage below runs
# the real entrypoint, unmodified, inside the real image, with a real volume,
# and replaces exactly two binaries: `tailscale` and `tailscaled`. Everything
# past the tailnet gate executes for real and has to reach BOOT COMPLETE: the
# privilege drop, the doorbell, `claudemon init`, `claudemon serve`, the
# readiness poll, `brain`, the hub, and the health watchdog.
#
# What it still cannot prove is listed in deploy/fly/RUNBOOK.md § "What a real
# machine still has to settle". Read that list rather than assuming green here
# means green there.
set -euo pipefail

cd "$(dirname "$0")/../.."   # repo root

BASE_TAG="${WKS_BASE_TAG:-workspacer-node-base:dev}"
HUB_TAG="${WKS_HUB_TAG:-workspacer-hub:dev}"
EXAMPLE_TAG="${WKS_EXAMPLE_TAG:-workspacer-node-example:dev}"
STAGE="${1:-all}"

pass() { printf '  \033[32mOK\033[0m   %s\n' "$*"; }
fail() { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAILED=1; }
section() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
FAILED=0

need() { command -v "$1" >/dev/null 2>&1 || { echo "preflight: $1 is required"; exit 1; }; }
need docker
need python3

# ---------------------------------------------------------------------------
# 1. BUILD
# ---------------------------------------------------------------------------
# Always rebuild, never trust the tag. `:dev` is a MUTABLE local tag: the image
# sitting under it may predate the last edit to entrypoint.sh, and nothing about
# looking at it would tell you. That is not hypothetical. On 2026-08-25 both
# `:dev` images on the machine that wrote this were built 38 minutes before the
# setpriv fix landed, and still carried the dead line. `fly deploy --local-only` for
# the hub resolves `FROM workspacer-node-base:dev` against whatever is on disk,
# so a stale base ships a machine that cannot boot.
#
# A no-op rebuild costs seconds against docker's layer cache. Do it every time.
if [ "$STAGE" = all ] || [ "$STAGE" = build ]; then
  section "BUILD: rebuilding both images (the :dev tag is mutable, never trust it)"

  # BuildKit writes its progress to stderr, so a bare >/dev/null still floods the
  # terminal. Capture both and print the log only when the build actually fails,
  # which is the one time it is worth reading.
  build() {
    local label="$1"; shift
    local log; log="$(mktemp)"
    if docker build "$@" >"$log" 2>&1; then
      pass "$label"
    else
      fail "$label (build log follows)"
      tail -40 "$log" | sed 's/^/      /'
    fi
    rm -f "$log"
  }

  build "base    $BASE_TAG" -f deploy/fly/node/Dockerfile -t "$BASE_TAG" .
  build "hub     $HUB_TAG" -f deploy/fly/hub/Dockerfile --build-arg WKS_BASE="$BASE_TAG" -t "$HUB_TAG" .
  build "example $EXAMPLE_TAG (proves the base extends)" \
    -f deploy/fly/node/example.Dockerfile --build-arg WKS_BASE="$BASE_TAG" \
    -t "$EXAMPLE_TAG" deploy/fly/node

  # The digests are what a downstream Dockerfile should pin. See
  # deploy/fly/node/BASE_IMAGE.md § "Identity and tagging".
  printf '  base digest: %s\n' "$(docker image inspect "$BASE_TAG" --format '{{.Id}}')"
fi

# ---------------------------------------------------------------------------
# 2. STATIC: the checks that read files rather than run them
# ---------------------------------------------------------------------------
if [ "$STAGE" = all ] || [ "$STAGE" = static ]; then
  section "STATIC: TOML, shell, Dockerfiles"

  python3 - <<'PY' && pass "both fly.toml parse, and their settings match the decisions"
import sys, tomllib
ok = True
def check(name, cond, why):
    global ok
    if not cond:
        print(f"    MISMATCH {name}: {why}"); ok = False

with open("deploy/fly/node/fly.toml", "rb") as f: n = tomllib.load(f)
with open("deploy/fly/hub/fly.toml", "rb") as f: h = tomllib.load(f)

# The decisions this deployment must not drift away from.
check("node region",   n["primary_region"] == "ord", "the volume pins the region permanently")
check("hub region",    h["primary_region"] == "ord", "hub and node should share a region")
check("node restart",  n["restart"][0]["policy"] == "on-failure",
      "fly.toml spells it on-failure; 'always' would defeat the sleep design")
check("hub restart",   h["restart"][0]["policy"] == "always",
      "the hub IS the wake path and has no legitimate stopped state")
check("node autostop", n["http_service"]["auto_stop_machines"] == "off",
      "only the hub may sleep the node")
check("node autostart", n["http_service"]["auto_start_machines"] is True,
      "the token-free wake backstop")
check("hub has no http_service", "http_service" not in h,
      "the hub is deliberately not publicly routable")
check("hub binds loopback", h["env"]["WKS_HUB_BIND"] == "127.0.0.1",
      "the Fly proxy cannot reach a loopback listener; that is the enforcement")
check("hub vm", h["vm"][0]["size"] == "shared-cpu-1x" and h["vm"][0]["memory"] == "1gb",
      "the budgeted always-on shape")
for name, cfg in (("node", n), ("hub", h)):
    for key in ("HOME", "WKS_DATA", "XDG_CONFIG_HOME"):
        check(f"{name} {key}", key in cfg["env"], "persistence depends on $HOME being on the volume")
    check(f"{name} XDG under HOME", cfg["env"]["XDG_CONFIG_HOME"].startswith(cfg["env"]["HOME"]),
          "config.yaml has two writers and they must land on one file")
sys.exit(0 if ok else 1)
PY

  ./deploy/fly/node/test-bootstrap.sh >/dev/null && pass "node bootstrap.sh: 63 assertions"
  ./deploy/fly/hub/test-bootstrap.sh  >/dev/null && pass "hub bootstrap.sh: 111 assertions"

  docker run --rm -v "$PWD/deploy/fly/node:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
    -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh verify-image.sh && pass "shellcheck node"
  docker run --rm -v "$PWD/deploy/fly/hub:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
    -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh && pass "shellcheck hub"

  for df in node/Dockerfile node/example.Dockerfile hub/Dockerfile; do
    docker run --rm -i hadolint/hadolint hadolint --failure-threshold info - \
      < "deploy/fly/$df" && pass "hadolint $df"
  done

  docker build --check -f deploy/fly/node/Dockerfile . >/dev/null 2>&1 && pass "docker build --check node"
  docker build --check -f deploy/fly/hub/Dockerfile  . >/dev/null 2>&1 && pass "docker build --check hub"
fi

# ---------------------------------------------------------------------------
# 3. CONTRACT: run the image's own rules against itself
# ---------------------------------------------------------------------------
if [ "$STAGE" = all ] || [ "$STAGE" = contract ]; then
  section "CONTRACT: the base image's own rules (BASE_IMAGE.md)"
  docker run --rm --entrypoint /usr/local/lib/wks/verify-image.sh "$BASE_TAG" >/dev/null \
    && pass "verify-image.sh passes inside the base"
  docker run --rm --entrypoint /usr/local/lib/wks/verify-image.sh "$EXAMPLE_TAG" >/dev/null \
    && pass "verify-image.sh passes inside the downstream example"

  vol="wks-preflight-empty-$$"
  docker volume create "$vol" >/dev/null
  docker run --rm -v "$vol:/data" -u 10001:10001 -e HOME=/data/home \
    --entrypoint /bin/bash "$BASE_TAG" -c /usr/local/lib/wks/test-bootstrap.sh >/dev/null \
    && pass "63 assertions again INSIDE the image, as wks, on an empty volume"
  docker volume rm "$vol" >/dev/null
fi

# ---------------------------------------------------------------------------
# 4. BOOT REHEARSAL: the stage that would have caught setpriv
# ---------------------------------------------------------------------------
if [ "$STAGE" = all ] || [ "$STAGE" = boot ]; then
  section "BOOT REHEARSAL: the real entrypoint, in the real image, on a real volume"

  shim="$(mktemp -d)"
  trap 'rm -rf "$shim"' EXIT

  cat >"$shim/tailscaled" <<'SHIM'
#!/bin/sh
# Stands in for kernel-mode tailscaled, which needs a tun device and a tailnet.
# This is the ONLY thing the rehearsal fakes, besides the CLI below.
mkdir -p /var/run/tailscale
while true; do sleep 3600; done
SHIM

  cat >"$shim/tailscale" <<'SHIM'
#!/bin/sh
sub=""
for a in "$@"; do case "$a" in --socket=*) continue;; esac; sub="$a"; break; done
case "$sub" in
  status)
    case "$*" in
      *--json*) printf '{"BackendState":"Running","Self":{"DNSName":"preflight.example.ts.net."}}\n' ;;
      *)        echo "100.99.99.9  preflight  shim  linux  -" ;;
    esac ;;
  ip)    echo "100.99.99.9" ;;
  up)    : ;;
  serve) : ;;
esac
exit 0
SHIM
  chmod +x "$shim/tailscale" "$shim/tailscaled"

  # Wait for BOOT COMPLETE, or dump the log and fail. A boot that stalls is as
  # much a failure as one that exits, so this is a timeout, not a sleep.
  rehearse() {
    local name="$1" tag="$2"; shift 2
    local vol="wks-rehearse-$name-$$" cid=""
    docker volume create "$vol" >/dev/null
    cid=$(docker run -d --rm \
      -v "$vol:/data" \
      -v "$shim/tailscale:/usr/local/bin/tailscale:ro" \
      -v "$shim/tailscaled:/usr/local/bin/tailscaled:ro" \
      -e TAILSCALE_AUTHKEY='tskey-auth-PREFLIGHT-SHIM' \
      "$@" "$tag")
    local waited=0 ok=0
    while [ "$waited" -lt 90 ]; do
      if docker logs "$cid" 2>&1 | grep -q 'BOOT COMPLETE'; then ok=1; break; fi
      docker inspect "$cid" >/dev/null 2>&1 || break   # container died
      sleep 2; waited=$((waited + 2))
    done
    if [ "$ok" = 1 ]; then
      pass "$name reached BOOT COMPLETE in ${waited}s (entrypoint ran end to end)"
    else
      fail "$name never reached BOOT COMPLETE (log follows)"
      docker logs "$cid" 2>&1 | tail -40 | sed 's/^/      /'
    fi
    docker rm -f "$cid" >/dev/null 2>&1 || true
    docker volume rm "$vol" >/dev/null 2>&1 || true
  }

  # The node's brain is pointed at a dead port on purpose: this stage proves the
  # BOOT, not the attach. It logs BOOT COMPLETE before the brain's first dial.
  rehearse node "$BASE_TAG" \
    -e HUB_BUS_URL='ws://127.0.0.1:59999/bus' \
    -e HUB_TOKEN='preflight-not-a-real-token'

  # The hub mints its own pairing credential on an empty volume and must reach
  # BOOT COMPLETE with no secrets at all.
  rehearse hub "$HUB_TAG"
fi

section "RESULT"
if [ "$FAILED" = 0 ]; then
  printf '  \033[32mpreflight green.\033[0m Nothing left that can be proved without a Fly account.\n'
  printf '  Next: deploy/fly/RUNBOOK.md, part B.\n\n'
else
  printf '  \033[31mpreflight FAILED.\033[0m Do not provision until this is green.\n\n'
  exit 1
fi
