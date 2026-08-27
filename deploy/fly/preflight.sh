#!/usr/bin/env bash
#
# preflight.sh: everything you can prove about a Fly deployment WITHOUT a Fly
# account, a tailnet, a credential or a single cent.
#
# Run this before you sit down to provision. It takes tens of minutes on a cold
# cache and about a minute on a warm one, it needs nothing but docker, and when
# it is green the only things left are the three that genuinely need a human.
#
#   ./deploy/fly/preflight.sh           # everything
#   ./deploy/fly/preflight.sh build     # just the images
#   ./deploy/fly/preflight.sh boot      # just the boot rehearsal
#   ./deploy/fly/preflight.sh artifact  # just the WKS_INSTALL=artifact path
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

  # The commit these source-mode images are built from. Nothing in the build
  # context can supply it — `**/.git` is excluded from both .dockerignores on
  # purpose — so it is passed in, and lands in the image's build-stamp. Without
  # it a locally built image stamps `commit=unknown`, which is honest but useless.
  SRC_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"

  build "base    $BASE_TAG" -f deploy/fly/node/Dockerfile \
    --build-arg WKS_SOURCE_SHA="$SRC_SHA" -t "$BASE_TAG" .
  build "hub     $HUB_TAG" -f deploy/fly/hub/Dockerfile --build-arg WKS_BASE="$BASE_TAG" \
    --build-arg WKS_SOURCE_SHA="$SRC_SHA" -t "$HUB_TAG" .
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

  ./deploy/fly/node/test-bootstrap.sh >/dev/null && pass "node bootstrap.sh: 106 assertions"
  ./deploy/fly/hub/test-bootstrap.sh  >/dev/null && pass "hub bootstrap.sh: 113 assertions"
  # The artifact install path, offline: curl speaks file://, so the real
  # download-verify-install runs against tarballs the suite builds. Everything
  # the ARTIFACT stage below cannot prove without docker is proved here.
  ./deploy/fly/test-fetch-release.sh >/dev/null && pass "fetch-release.sh: 51 assertions (artifact mode, offline)"

  docker run --rm -v "$PWD/deploy/fly/node:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
    -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh verify-image.sh && pass "shellcheck node"
  docker run --rm -v "$PWD/deploy/fly/hub:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
    -s bash -S style entrypoint.sh bootstrap.sh test-bootstrap.sh && pass "shellcheck hub"
  docker run --rm -v "$PWD/deploy/fly:/mnt:ro" -w /mnt koalaman/shellcheck:stable \
    -s bash -S style fetch-release.sh write-build-stamp.sh test-fetch-release.sh && pass "shellcheck shared (fetch-release, write-build-stamp)"

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
    && pass "106 assertions again INSIDE the image, as wks, on an empty volume"
  docker volume rm "$vol" >/dev/null

  # The build stamp, read out of the finished image rather than out of the build
  # log. Until this file existed there was NO honest way to ask a box which
  # commit it runs: `workspacer`, `hub` and `brain` have no --version, and
  # `claudemon --version` prints a Cargo version that has not moved in years.
  stamp_of() { docker run --rm --entrypoint cat "$1" "${2:-/usr/local/share/workspacer/build-stamp}" 2>/dev/null; }
  base_stamp="$(stamp_of "$BASE_TAG")"
  if grep -q '^install=source$' <<<"$base_stamp"; then
    pass "the base image's build-stamp says install=source"
  else
    fail "the base image's build-stamp does not say install=source: $base_stamp"
  fi
  if grep -q "^commit=$(git rev-parse HEAD 2>/dev/null || echo unknown)\$" <<<"$base_stamp"; then
    pass "and names the commit it was actually built from"
  else
    fail "the base stamp's commit is not this worktree's HEAD: $base_stamp"
  fi
  if grep -q '^component=hub$' <<<"$(stamp_of "$HUB_TAG" /usr/local/share/workspacer/build-stamp.hub)"; then
    pass "the hub layer stamps SEPARATELY, beside the base's rather than over it"
  else
    fail "the hub image has no /usr/local/share/workspacer/build-stamp.hub"
  fi
fi

# ---------------------------------------------------------------------------
# 3b. ARTIFACT: the install-from-release path, with no release and no network
# ---------------------------------------------------------------------------
# WKS_INSTALL=artifact installs the box from a published `workspacer-server-*`
# bundle instead of compiling the tree — which is the whole point (11 seconds
# instead of ten minutes), and also the whole risk: a mutable `nightly` tag means
# "the download worked" says nothing about what is now in the image.
#
# This stage proves the wiring without publishing anything. It takes the files
# the SOURCE build just installed, repackages them as the bundle the release
# workflow produces, serves them over the docker bridge, and rebuilds the image
# in artifact mode. Then it checks the two things that matter: the drift guard
# fires on the wrong tag, and a correct artifact image passes the SAME
# verify-image.sh the source one does.
#
# It needs the base image, so it runs after BUILD. If the build container cannot
# reach the fixture server (an unusual docker network setup), it SKIPS loudly
# rather than failing — test-fetch-release.sh in the STATIC stage already proves
# the logic itself, offline, with 51 assertions.
if [ "$STAGE" = all ] || [ "$STAGE" = artifact ]; then
  section "ARTIFACT: WKS_INSTALL=artifact, against a fixture release built from this tree"

  docker image inspect "$BASE_TAG" >/dev/null 2>&1 || {
    echo "  (needs $BASE_TAG — run the build stage first)"; exit 1
  }

  af_dir="$(mktemp -d)"
  af_srv=""
  cleanup_artifact() {
    [ -n "$af_srv" ] && docker rm -f "$af_srv" >/dev/null 2>&1
    rm -rf "$af_dir"
  }
  trap cleanup_artifact EXIT

  # Repackage the source image's own files into the shape
  # .github/workflows/release.yml ships. Using the built image rather than the
  # worktree means the fixture is real binaries, not stubs, so the artifact image
  # this produces is one that could actually boot.
  af_tag="preflight-fixture"
  af_root="$af_dir/releases/$af_tag"
  mkdir -p "$af_root" "$af_dir/stage/workspacer-server"
  af_cid="$(docker create "$BASE_TAG")"
  for b in workspacer brain mcp claudemon; do
    docker cp "$af_cid:/usr/local/bin/$b" "$af_dir/stage/workspacer-server/$b" >/dev/null
  done
  docker cp "$af_cid:/usr/local/share/workspacer/build-stamp" "$af_dir/stage/workspacer-server/build-stamp" >/dev/null
  docker rm "$af_cid" >/dev/null
  # `hub` and web/ come off the hub image when there is one, so the hub's own
  # artifact require-list has something to find.
  if docker image inspect "$HUB_TAG" >/dev/null 2>&1; then
    af_hcid="$(docker create "$HUB_TAG")"
    docker cp "$af_hcid:/usr/local/bin/hub" "$af_dir/stage/workspacer-server/hub" >/dev/null
    docker cp "$af_hcid:/usr/local/share/workspacer/web" "$af_dir/stage/workspacer-server/web" >/dev/null 2>&1 || true
    docker rm "$af_hcid" >/dev/null
  fi
  # Restamp as a RELEASE bundle under the fixture tag: an image built from this
  # must be able to say it came from a release, not from this worktree.
  WKS_STAMP_COMPONENT=server WKS_STAMP_INSTALL=release \
  WKS_STAMP_VERSION=preflight-fixture WKS_STAMP_TAG="$af_tag" \
  WKS_STAMP_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)" \
  WKS_STAMP_PLATFORM=linux-x64 WKS_STAMP_RUN=preflight \
    ./deploy/fly/write-build-stamp.sh "$af_dir/stage/workspacer-server/build-stamp" >/dev/null
  tar -C "$af_dir/stage" -czf "$af_root/workspacer-server-linux-x64.tar.gz" workspacer-server
  # The SAME bundle, served under a SECOND tag. This is what a rolled tag, a
  # stale mirror or a moved v* actually looks like: the download succeeds and the
  # bytes are wrong. Pointing the guard at a tag that simply 404s would prove only
  # that curl works.
  mkdir -p "$af_dir/releases/preflight-wrong-tag"
  cp "$af_root/workspacer-server-linux-x64.tar.gz" "$af_dir/releases/preflight-wrong-tag/"

  # Serve it on the docker bridge gateway, using the image already on the machine
  # (it carries busybox-static for the node's own doorbell) so this pulls nothing.
  af_gw="$(docker network inspect bridge -f '{{range .IPAM.Config}}{{.Gateway}}{{end}}' 2>/dev/null)"
  af_port=$((18700 + ($$ % 200)))
  if [ -n "$af_gw" ]; then
    af_srv="$(docker run -d --rm -v "$af_dir/releases:/srv:ro" \
      -p "$af_gw:$af_port:8080" --entrypoint busybox "$BASE_TAG" \
      httpd -f -p "0.0.0.0:8080" -h /srv 2>/dev/null || true)"
  fi
  af_url="http://$af_gw:$af_port"
  if [ -z "$af_srv" ] || ! curl -sfI --max-time 5 "$af_url/$af_tag/workspacer-server-linux-x64.tar.gz" >/dev/null 2>&1; then
    printf '  \033[33mSKIP\033[0m artifact stage: no fixture server reachable at %s.\n' "$af_url"
    printf '       The logic is covered offline by test-fetch-release.sh in the STATIC stage.\n'
  else
    af_build() {
      local label="$1" want_rc="$2"; shift 2
      local log rc=0; log="$(mktemp)"
      # `|| rc=$?` and not a bare call: `set -e` is on, and HALF the calls here
      # are SUPPOSED to fail — a drift guard that never fires proves nothing.
      docker build -f deploy/fly/node/Dockerfile \
        --build-arg WKS_INSTALL=artifact \
        --build-arg WKS_RELEASE_BASE_URL="$af_url" \
        "$@" -t workspacer-node-base:artifact-preflight . >"$log" 2>&1 || rc=$?
      if [ "$rc" = "$want_rc" ]; then pass "$label"; else
        fail "$label (rc=$rc, want $want_rc; log follows)"; tail -25 "$log" | sed 's/^/      /'
      fi
      AF_LOG="$log"
    }

    # THE DRIFT GUARD, which is the reason this mode is allowed to exist at all.
    af_build "DRIFT GUARD: a bundle that downloads fine but claims another tag FAILS the build" 1 \
      --build-arg WKS_RELEASE_TAG=preflight-wrong-tag
    if grep -qF 'RELEASE DRIFT' "$AF_LOG"; then
      pass "and the build log names it as release drift"
    else
      fail "the failing build did not print RELEASE DRIFT"
    fi
    rm -f "$AF_LOG"

    af_build "COMMIT GUARD: the right tag with the wrong sha FAILS the build" 1 \
      --build-arg WKS_RELEASE_TAG="$af_tag" \
      --build-arg WKS_RELEASE_SHA=0000000000000000000000000000000000000000
    if grep -qF 'COMMIT DRIFT' "$AF_LOG"; then
      pass "and names the commit mismatch"
    else
      fail "the failing build did not print COMMIT DRIFT"
    fi
    rm -f "$AF_LOG"

    af_build "an artifact image builds when tag AND sha both match" 0 \
      --build-arg WKS_RELEASE_TAG="$af_tag" \
      --build-arg WKS_RELEASE_SHA="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    # NO TOOLCHAIN. This is the claim the mode is sold on, so it is asserted from
    # the build log rather than assumed from the Dockerfile's shape.
    if grep -qE '^#[0-9]+ \[(gobuild|rustbuild)' "$AF_LOG"; then
      fail "artifact mode still ran a Go or Rust builder stage"
    else
      pass "NO TOOLCHAIN: neither the gobuild nor the rustbuild stage ran"
    fi
    rm -f "$AF_LOG"

    docker run --rm --entrypoint /usr/local/lib/wks/verify-image.sh \
      workspacer-node-base:artifact-preflight >/dev/null \
      && pass "verify-image.sh passes inside the artifact image, same as the source one"

    af_stamp="$(docker run --rm --entrypoint cat workspacer-node-base:artifact-preflight \
      /usr/local/share/workspacer/build-stamp 2>/dev/null)"
    if grep -q "^install=release$" <<<"$af_stamp"; then
      pass "and its stamp says install=release, not source — provenance survived the copy"
    else
      fail "the artifact image's stamp does not say install=release: $af_stamp"
    fi
    if grep -q "^tag=$af_tag\$" <<<"$af_stamp"; then
      pass "and names the release tag it came from"
    else
      fail "the artifact image's stamp does not name the tag: $af_stamp"
    fi

    # The claim the whole runbook rests on: the boot log SAYS this, without a
    # shell on the machine. `fly logs` is all an operator gets on a box that
    # died, so an identity only readable by exec is not an identity.
    boot_line="$(docker run --rm --entrypoint /bin/bash workspacer-node-base:artifact-preflight -c \
      'WKS_BUILD_STAMP=/usr/local/share/workspacer/build-stamp; printf "  build: %s\n" "$(tr "\n" " " <"$WKS_BUILD_STAMP")"' 2>/dev/null)"
    if grep -qF "tag=$af_tag" <<<"$boot_line"; then
      pass "the entrypoint's stamp line renders on one line: ${boot_line# }"
    else
      fail "the entrypoint's stamp line did not render: $boot_line"
    fi

    docker rmi workspacer-node-base:artifact-preflight >/dev/null 2>&1 || true
  fi

  cleanup_artifact
  af_srv=""
  trap - EXIT
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

  # A rehearsal's log is worth more than its exit status, so it is kept: the
  # named assertions below read it. $REHEARSE_LOG is the last one written, and
  # $REHEARSE_VOL is the volume, so a case that needs a SECOND boot on the same
  # state can ask for one.
  REHEARSE_LOG=""
  REHEARSE_VOL=""

  # Wait for BOOT COMPLETE, or dump the log and fail. A boot that stalls is as
  # much a failure as one that exits, so this is a timeout, not a sleep.
  # keep=1 leaves the volume behind for a follow-up boot; the caller drops it.
  rehearse() {
    local name="$1" tag="$2" keep="$3"; shift 3
    local vol="${REHEARSE_VOL:-}" cid=""
    if [ -z "$vol" ] || [ "$keep" != "reuse" ]; then
      vol="wks-rehearse-$name-$$-$RANDOM"
      docker volume create "$vol" >/dev/null
    fi
    REHEARSE_VOL="$vol"
    REHEARSE_LOG="$(mktemp)"
    cid=$(docker run -d --rm \
      -v "$vol:/data" \
      -v "$shim/tailscale:/usr/local/bin/tailscale:ro" \
      -v "$shim/tailscaled:/usr/local/bin/tailscaled:ro" \
      -e TAILSCALE_AUTHKEY='tskey-auth-PREFLIGHT-SHIM' \
      "$@" "$tag")
    local waited=0 ok=0
    while [ "$waited" -lt 120 ]; do
      if docker logs "$cid" 2>&1 | grep -q 'BOOT COMPLETE'; then ok=1; break; fi
      docker inspect "$cid" >/dev/null 2>&1 || break   # container died
      sleep 2; waited=$((waited + 2))
    done
    # Give the children a moment to say their piece. The brain's config read,
    # which is what the false-alarm assertion below is about, happens AFTER the
    # entrypoint has already logged BOOT COMPLETE.
    sleep 6
    docker logs "$cid" >"$REHEARSE_LOG" 2>&1 || true
    if [ "$ok" = 1 ]; then
      pass "$name reached BOOT COMPLETE in ${waited}s (entrypoint ran end to end)"
    else
      fail "$name never reached BOOT COMPLETE (log follows)"
      tail -40 "$REHEARSE_LOG" | sed 's/^/      /'
    fi
    docker rm -f "$cid" >/dev/null 2>&1 || true
    if [ "$keep" != "keep" ]; then
      docker volume rm "$vol" >/dev/null 2>&1 || true
      REHEARSE_VOL=""
    fi
  }

  # in_log / not_in_log: named assertions against the boot that just ran. These
  # exist because a clean build proves ASSEMBLY, not BOOT, and BOOT COMPLETE on
  # its own proves the entrypoint finished, not that it did the right things.
  in_log()     { if grep -qF "$1" "$REHEARSE_LOG"; then pass "$2"; else fail "$2"; fi; }
  not_in_log() { if grep -qF "$1" "$REHEARSE_LOG"; then fail "$2"; else pass "$2"; fi; }
  # Same, ignoring the replayed previous boot. Lines quoted from an older log
  # carry a "  | " or "  ! " marker precisely so this distinction is possible.
  not_in_own_log() {
    if grep -v '^  [|!] ' "$REHEARSE_LOG" | grep -qF "$1"; then fail "$2"; else pass "$2"; fi
  }

  # The node's brain is pointed at a dead port on purpose: this stage proves the
  # BOOT, not the attach. It logs BOOT COMPLETE before the brain's first dial.
  # The volume is KEPT so the second boot below can replay the first one's log.
  rehearse node "$BASE_TAG" keep \
    -e HUB_BUS_URL='ws://127.0.0.1:59999/bus' \
    -e HUB_TOKEN='preflight-not-a-real-token'

  # ---- the false alarm, asserted against a real brain on a real volume ----
  # The node used to print `brain: STATE LOSS: …/config.yaml is missing` on every
  # genuinely first boot, because bootstrap.sh pre-creates sibling directories
  # under ~/.config/workspacer and internal/statelost counted an empty directory
  # as evidence somebody had run there. This is the assertion that says it is
  # gone, and it is a real brain reading a real volume, not a unit test.
  not_in_log "brain: STATE LOSS" \
    "FALSE ALARM GONE: a genuinely first boot prints no brain STATE LOSS line"
  in_log "state guard: no losses detected" \
    "the node's own state guard ran and found nothing lost on an empty volume"
  in_log "no previous boot log" \
    "the first boot says there is no previous boot log to replay"
  in_log "brain: scope=full" "the brain still started and registered its capabilities"

  # ---- the previous boot log reaches fly logs on the NEXT boot ----
  # The finding this closes: /data/logs/boot.log is readable only through a shell
  # on the machine, and a node that fails to boot does not stay up long enough to
  # give you one. So the previous boot is replayed to stdout.
  rehearse node "$BASE_TAG" reuse \
    -e HUB_BUS_URL='ws://127.0.0.1:59999/bus' \
    -e HUB_TOKEN='preflight-not-a-real-token'
  in_log "PREVIOUS BOOT, replayed to stdout" \
    "the second boot replays the first boot's log to stdout, where fly logs can see it"
  in_log "  | " "the replayed lines are marked so they cannot be mistaken for this boot"
  in_log "verdict: the previous boot COMPLETED" \
    "the replay leads with a verdict, not just a tail: the tail alone is runtime chatter"
  not_in_log "brain: STATE LOSS" "still no false alarm on the second boot"
  docker volume rm "$REHEARSE_VOL" >/dev/null 2>&1 || true
  REHEARSE_VOL=""

  # ---- the two findings proving each other ----
  # A state-guard refusal is the boot failure with the SHORTEST window: the
  # entrypoint dies about a second in, so `fly ssh console` never gets a machine
  # to attach to and the refusal on the volume is unreadable. That is only an
  # acceptable design if the NEXT boot carries it out. This runs the whole
  # sequence for real, plant a credential, boot so the marker is recorded, take
  # the credential away, watch the node refuse, then boot again and read the
  # refusal off stdout.
  section "REFUSAL: a node that will not boot still tells you why, on the next try"

  refusal_vol="wks-rehearse-refusal-$$"
  docker volume create "$refusal_vol" >/dev/null

  # Edit the volume the way a snapshot restore or a stray rm would. Runs as root
  # because a Fly volume mounts root-owned and bootstrap.sh is what fixes that.
  prep_vol() {
    docker run --rm -v "$refusal_vol:/data" --entrypoint /bin/sh "$BASE_TAG" -c "$1" >/dev/null 2>&1
  }

  # Boot once and capture the log whether it succeeds or dies. No --rm: a boot
  # that exits would take its own log with it, which is the exact failure this
  # section is about.
  boot_once() {
    local cid waited=0
    REHEARSE_LOG="$(mktemp)"
    cid=$(docker run -d \
      -v "$refusal_vol:/data" \
      -v "$shim/tailscale:/usr/local/bin/tailscale:ro" \
      -v "$shim/tailscaled:/usr/local/bin/tailscaled:ro" \
      -e TAILSCALE_AUTHKEY='tskey-auth-PREFLIGHT-SHIM' \
      -e HUB_BUS_URL='ws://127.0.0.1:59999/bus' \
      -e HUB_TOKEN='preflight-not-a-real-token' \
      "$BASE_TAG")
    while [ "$waited" -lt 60 ]; do
      [ "$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null)" = "true" ] || break
      docker logs "$cid" 2>&1 | grep -q 'BOOT COMPLETE' && break
      sleep 1; waited=$((waited + 1))
    done
    sleep 2
    docker logs "$cid" >"$REHEARSE_LOG" 2>&1 || true
    BOOT_EXIT="$(docker inspect -f '{{.State.ExitCode}}' "$cid" 2>/dev/null || echo unknown)"
    BOOT_RUNNING="$(docker inspect -f '{{.State.Running}}' "$cid" 2>/dev/null || echo unknown)"
    docker rm -f "$cid" >/dev/null 2>&1 || true
  }

  prep_vol 'mkdir -p /data/home/.claude && printf %s "{\"claudeAiOauth\":{\"accessToken\":\"x\"}}" > /data/home/.claude/.credentials.json'
  boot_once
  in_log "BOOT COMPLETE" "a node WITH a Claude credential boots normally"
  in_log "state guard: no losses detected" "and the guard records that the credential was here"

  prep_vol 'rm -f /data/home/.claude/.credentials.json'
  boot_once
  if [ "$BOOT_RUNNING" = "false" ] && [ "$BOOT_EXIT" != "0" ]; then
    pass "GUARD FIRES ON A GENUINE LOSS: taking the credential away stops the node booting (exit $BOOT_EXIT)"
  else
    fail "the node booted anyway after its Claude credential vanished (running=$BOOT_RUNNING exit=$BOOT_EXIT)"
  fi
  in_log "STATE LOSS" "the refusal names it as state loss"
  in_log "REFUSING TO START" "and refuses rather than coming up to hang every session"
  not_in_own_log "BOOT COMPLETE" \
    "the boot did NOT complete, and the replayed previous boot cannot be mistaken for one that did"

  # The one that matters. Everything above is on a volume nobody can reach.
  boot_once
  in_log "PREVIOUS BOOT, replayed" "the next attempt replays the boot that failed"
  in_log "DIED DURING STARTUP" "and leads with the verdict, so no log-reading is required to get it"
  in_log "  ! " "and surfaces the refusal lines from anywhere in that log, not only its tail"
  if grep -A200 'PREVIOUS BOOT, replayed' "$REHEARSE_LOG" | grep -qF 'REFUSING TO START'; then
    pass "THE REFUSAL REACHES fly logs: the reason the node would not boot is on stdout of the next boot"
  else
    fail "the replayed block does not carry the refusal"
  fi

  docker volume rm "$refusal_vol" >/dev/null 2>&1 || true

  # ---- last-exit.json is CONSUMED, so a hard kill is not reported as a sleep ----
  # last-exit.json is written by a trap, and the exits worth knowing about run no
  # trap. Left in place, the record from an earlier clean sleep is re-reported
  # against the run that was killed: the exact ambiguity the file exists to
  # resolve, answered backwards, and undetectably so because the reader drops the
  # bootId. This runs the real sequence: sleep cleanly, boot, then get SIGKILLed,
  # then boot again and check what the machine says about how it died.
  section "LAST EXIT: a run killed without warning is not reported as a clean sleep"

  le_vol="wks-rehearse-lastexit-$$"
  docker volume create "$le_vol" >/dev/null
  LE_CID=""

  le_boot() {
    LE_CID=$(docker run -d \
      -v "$le_vol:/data" \
      -v "$shim/tailscale:/usr/local/bin/tailscale:ro" \
      -v "$shim/tailscaled:/usr/local/bin/tailscaled:ro" \
      -e TAILSCALE_AUTHKEY='tskey-auth-PREFLIGHT-SHIM' \
      -e HUB_BUS_URL='ws://127.0.0.1:59999/bus' \
      -e HUB_TOKEN='preflight-not-a-real-token' \
      "$BASE_TAG")
    local waited=0
    while [ "$waited" -lt 60 ]; do
      docker logs "$LE_CID" 2>&1 | grep -q 'BOOT COMPLETE' && break
      sleep 1; waited=$((waited + 1))
    done
    REHEARSE_LOG="$(mktemp)"
    docker logs "$LE_CID" >"$REHEARSE_LOG" 2>&1 || true
  }

  le_boot
  # SIGTERM: the trap runs, the record is written. This is a clean sleep, the
  # shape the hub produces with nodes.sleep.
  docker stop -t 60 "$LE_CID" >/dev/null 2>&1 || true
  docker rm -f "$LE_CID" >/dev/null 2>&1 || true

  le_boot
  in_log "previous run ended:" "the next boot reports how the previous run ended"
  in_log "signal-TERM" "and a deliberate stop is reported as signal-TERM"
  if docker exec "$LE_CID" test -f /data/state/last-exit.consumed.json 2>/dev/null &&
    ! docker exec "$LE_CID" test -f /data/state/last-exit.json 2>/dev/null; then
    pass "CONSUMED: the record was renamed once logged, so it cannot be reported twice"
  else
    fail "last-exit.json is still in place after being reported: it will be re-reported"
  fi

  # SIGKILL: no trap, nothing written. The bug this closes is that the record
  # above would otherwise still be sitting there, and this boot would report it.
  docker kill -s KILL "$LE_CID" >/dev/null 2>&1 || true
  docker rm -f "$LE_CID" >/dev/null 2>&1 || true

  le_boot
  not_in_own_log "signal-TERM" \
    "A HARD KILL IS NOT REPORTED AS A CLEAN SLEEP: the stale signal-TERM is not reused"
  in_log "KILLED WITHOUT WARNING" "and the boot says a killed run writes no record, rather than inventing one"
  docker rm -f "$LE_CID" >/dev/null 2>&1 || true
  docker volume rm "$le_vol" >/dev/null 2>&1 || true

  # ---- the hub does not run shell for the node ----
  # The node attaches with an operator-tier token; operator tier is `trusted`;
  # jobsTrusted is a bare IsTrusted(). So jobs.upsert + jobs.run would give the
  # node /bin/sh in the hub's own environment: the one holding $FLY_API_TOKEN,
  # on the volume holding nodes.json and remote-token. The entrypoint passes an
  # empty --jobs-file, which is the documented off switch. Asserted BOTH ways:
  # the flag disables it, AND the same binary in the same image still registers
  # jobs.* when the flag is non-empty, so this is the flag doing the work and not
  # a method that never existed.
  section "JOBS: the hub runs shell for nobody, because the subsystem is off"

  jobs_vol="wks-rehearse-jobs-$$"
  docker volume create "$jobs_vol" >/dev/null
  jobs_cid=$(docker run -d \
    -v "$jobs_vol:/data" \
    -v "$shim/tailscale:/usr/local/bin/tailscale:ro" \
    -v "$shim/tailscaled:/usr/local/bin/tailscaled:ro" \
    -e TAILSCALE_AUTHKEY='tskey-auth-PREFLIGHT-SHIM' \
    "$HUB_TAG")
  jw=0
  while [ "$jw" -lt 90 ]; do
    docker logs "$jobs_cid" 2>&1 | grep -q 'BOOT COMPLETE' && break
    sleep 2; jw=$((jw + 2))
  done

  jobs_env=(-e HOME=/data/home -e XDG_CONFIG_HOME=/data/home/.config)
  jobs_out="$(docker exec "${jobs_env[@]}" "$jobs_cid" workspacer jobs list 2>&1)" && jobs_rc=0 || jobs_rc=$?
  if [ "$jobs_rc" != 0 ] && printf '%s' "$jobs_out" | grep -qF 'no provider for jobs.list'; then
    pass "JOBS ARE OFF: \`workspacer jobs list\` against the booted hub gets no provider for jobs.list"
  else
    fail "jobs.list answered on the hub (rc=$jobs_rc): $jobs_out"
  fi
  if docker exec "$jobs_cid" sh -c 'tr "\0" " " </proc/$(pgrep -x hub)/cmdline' 2>/dev/null |
    grep -qF -- '--jobs-file'; then
    pass "and the running hub's argv carries --jobs-file with an empty value"
  else
    fail "the running hub was not started with --jobs-file"
  fi

  # The negative control. Without it, "no provider" could mean the method never
  # existed rather than that the flag removed it.
  docker exec -d "${jobs_env[@]}" -e HUB_TOKEN=preflight-negative-control "$jobs_cid" \
    hub --addr 127.0.0.1:7999 --brain-scope off --jobs-file /tmp/negative-control-jobs.json \
    >/dev/null 2>&1 || true
  sleep 4
  if docker exec "${jobs_env[@]}" -e HUB_TOKEN=preflight-negative-control "$jobs_cid" \
    workspacer jobs list --hub-port 7999 >/dev/null 2>&1; then
    pass "NEGATIVE CONTROL: the same binary DOES register jobs.list when --jobs-file is non-empty"
  else
    fail "the negative control hub did not answer jobs.list: this test proves nothing as written"
  fi

  docker rm -f "$jobs_cid" >/dev/null 2>&1 || true
  docker volume rm "$jobs_vol" >/dev/null 2>&1 || true

  # The hub mints its own pairing credential on an empty volume and must reach
  # BOOT COMPLETE with no secrets at all.
  rehearse hub "$HUB_TAG" drop

  # ---- the end-to-end probe RUNS, and cannot fail a boot ----
  # The shimmed tailscale reports a MagicDNS name that resolves nowhere, so the
  # probe takes its failure path, which is the path that matters. A probe that
  # could stop a boot would be the setpriv mistake again with better intentions,
  # and this is the assertion that proves it cannot.
  in_log "probing https://preflight.example.ts.net/health" \
    "the hub actually issues the end-to-end request (executed, not parsed)"
  in_log "did NOT answer within" "and reports the failure loudly"
  in_log "this is the boot continuing, not failing" \
    "and says so in the log, so the operator is not hunting a crash"
  in_log "tailscale cert preflight.example.ts.net" "the failure carries a triage list, not just a status"
  in_log "BOOT COMPLETE" "NON-FATAL PROVEN: the boot completed anyway, after the probe failed"
fi

section "RESULT"
if [ "$FAILED" = 0 ]; then
  printf '  \033[32mpreflight green.\033[0m Nothing left that can be proved without a Fly account.\n'
  printf '  Next: deploy/fly/RUNBOOK.md, part B.\n\n'
else
  printf '  \033[31mpreflight FAILED.\033[0m Do not provision until this is green.\n\n'
  exit 1
fi
