#!/usr/bin/env bash
#
# bootstrap.sh — prepare the always-on hub's Fly volume, and decide, for every
# create-once file on it, whether this is a FIRST RUN or STATE LOSS.
#
# ---------------------------------------------------------------------------
# WHY THIS FILE IS DIFFERENT FROM THE NODE'S
# ---------------------------------------------------------------------------
# The node's bootstrap prepares a volume. This one does that too — same rule,
# no symlinks, $HOME on the volume — but the interesting half is the second job,
# and it exists because of one sentence:
#
#   *** A HUB THAT RESTARTS WITHOUT ITS STATE DOES NOT FAIL. IT SILENTLY
#       BECOMES A DIFFERENT HUB. ***
#
# Three files decide the hub's identity and every one of them is loaded by code
# shaped "read it; if it is not there, make a new one":
#
#   remote-token   the pairing credential. Re-minted = every client, phone and
#                  federation peer is refused, while the banner says healthy.
#   vapid.json     a generate-once Web Push keypair. Regenerated = every push
#                  subscription is dead, and every phone still believes it is
#                  subscribed.
#   nodes.json     the node registry. MISSING IS NOT AN ERROR — the hub logs
#                  nothing and simply never registers nodes.list/nodes.wake.
#                  A hub that loses this comes up perfectly healthy with no way
#                  to wake anything, which on this machine is the whole point.
#
# The upstream answer is internal/statelost: is the directory around the missing
# file empty (nobody ever ran here) or does it still hold the rest of the state
# (something took this file away)? `workspacer serve` uses it to REFUSE rather
# than re-mint a remote-token. But the `hub` binary this machine actually runs
# does not go through that path at all — it takes its token from $HUB_TOKEN —
# and nothing anywhere applies the rule to nodes.json.
#
# So the rule is applied HERE, before the hub starts, with better evidence:
# a per-file marker under $WKS_DATA/state/seen/. "This file has existed on this
# volume before" is a FACT; "the directory has something else in it" is an
# INFERENCE, and one that is only sound when running the hub guarantees the file
# exists. That is true of remote-token and of nothing else here, so remote-token
# consults both signals and every other file consults the marker alone. The
# reasoning, and the two bugs that produced it, are at bs_dir_had_other/bs_guard.
#
# ---------------------------------------------------------------------------
# THE SYMLINK RULE, INHERITED UNCHANGED
# ---------------------------------------------------------------------------
# Persist DIRECTORIES, never individual files. Every atomic writer in this stack
# writes a sibling temp file and rename(2)s it over the target; rename() replaces
# the DIRECTORY ENTRY, so a FILE symlink is destroyed and replaced by a regular
# file on the ephemeral rootfs. It works for the rest of that boot and the data
# is gone on the next restart. This script therefore creates no symlinks at all:
# $HOME itself is on the volume, so everything dotfile-shaped is persistent by
# construction. test-bootstrap.sh asserts that mechanically, and demonstrates the
# rename() mechanism executably so it is not folklore.
#
# ---------------------------------------------------------------------------
# CONTRACT
# ---------------------------------------------------------------------------
# Idempotent. Correct on an EMPTY volume (first boot), a POPULATED one (every
# later boot), and a DAMAGED one (repairs and says so). A pure function of the
# environment below, so it runs against a temp directory standing in for /data.
#
#   WKS_DATA                  volume root                        (default /data)
#   WKS_HOME                  $HOME on the volume                (default $WKS_DATA/home)
#   WKS_UID / WKS_GID         uid:gid that must own the volume   (default 10001:10001)
#   WKS_REQUIRE_MOUNT         1 = refuse unless $WKS_DATA is a real mountpoint
#   WKS_SKIP_CHOWN            1 = skip ownership reconciliation (local tests, non-root)
#   HUB_TOKEN                 the pairing credential, from `fly secrets set`
#   WORKSPACER_ALLOW_NEW_TOKEN=1  accept a NEW pairing credential after loss
#                             (the env twin of `workspacer serve --allow-new-token`)
#   WKS_ALLOW_TOKEN_CHANGE=1  accept $HUB_TOKEN differing from the persisted one
#   WKS_ALLOW_STATE_LOSS=1    downgrade every refusal below to a warning
#
# Exit 0 = the volume is ready and the hub may start.
# Exit non-zero = do NOT start the hub. The message says which file and why.
#
set -euo pipefail

WKS_DATA="${WKS_DATA:-/data}"
WKS_HOME="${WKS_HOME:-${WKS_DATA}/home}"
WKS_UID="${WKS_UID:-10001}"
WKS_GID="${WKS_GID:-10001}"
WKS_REQUIRE_MOUNT="${WKS_REQUIRE_MOUNT:-1}"
WKS_SKIP_CHOWN="${WKS_SKIP_CHOWN:-0}"
WKS_ALLOW_STATE_LOSS="${WKS_ALLOW_STATE_LOSS:-0}"
WKS_ALLOW_TOKEN_CHANGE="${WKS_ALLOW_TOKEN_CHANGE:-0}"
WORKSPACER_ALLOW_NEW_TOKEN="${WORKSPACER_ALLOW_NEW_TOKEN:-0}"

# Bumped when the layout below changes in a way an existing volume must be told
# about. Recorded on the volume so a later boot can tell "first boot" from "boot
# against a volume built by an older image".
WKS_LAYOUT_VERSION=1

CFG="$WKS_HOME/.config"                  # == $XDG_CONFIG_HOME
CFG_WKS="$CFG/workspacer"                # remote-token, tokens.json, peers.json, nodes.json, config.yaml
CFG_HUB="$CFG/workspacer-hub"            # vapid.json, push-subscriptions.json, jobs*.json, layout.json
SEEN="$WKS_DATA/state/seen"              # one marker per create-once file ever observed

bs_log()  { printf '%s bootstrap: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
bs_warn() { bs_log "WARNING: $*" >&2; }
bs_die()  { bs_log "FATAL: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 0. Refuse to run without the volume.
# ---------------------------------------------------------------------------
# Running on the ephemeral rootfs is the worst outcome available, and on THIS
# machine it is worse than on the node: tailscaled mints a new node key, the
# pairing credential is re-minted, the VAPID keypair is regenerated, and the node
# registry is simply absent — so the hub comes up healthy, refuses every client
# it has ever paired with, kills every push subscription, and can wake nothing.
# Every one of those is silent. Fail here instead.
bs_is_mountpoint() {
  awk -v d="$1" '$2 == d { found = 1 } END { exit !found }' /proc/self/mounts
}

if [ ! -d "$WKS_DATA" ]; then
  bs_die "$WKS_DATA does not exist — the volume is not mounted. Refusing to run on the rootfs."
fi
if [ "$WKS_REQUIRE_MOUNT" = "1" ] && ! bs_is_mountpoint "$WKS_DATA"; then
  bs_die "$WKS_DATA exists but is not a mountpoint — the volume failed to attach. Refusing to run on the rootfs."
fi

# ---------------------------------------------------------------------------
# 1. The layout.
# ---------------------------------------------------------------------------
# Verified against the source, not copied from the node. What is NOT here is as
# deliberate as what is: this machine runs `hub` and nothing else — no claudemon,
# no brain, no Claude Code — so it holds NO ~/.claude, NO ~/.claude.json, NO
# ~/.codex, NO ~/.ssh, NO repo checkouts and NO toolchain caches. The always-on,
# credential-holding machine is also the one with no agent credentials on it,
# and that is the right way round. RUNBOOK.md §7 has the path-by-path table.
bs_dirs=(
  # bookkeeping + the boot log, which must exist before anything else runs
  "$WKS_DATA/logs"
  "$WKS_DATA/state"
  "$SEEN"

  # $HOME. Persistent by construction; no symlinks anywhere below this point.
  "$WKS_HOME"
  "$CFG"
  # authtoken.ConfigDir() — $XDG_CONFIG_HOME/workspacer. Home of remote-token
  # (cmd/workspacer/token.go), tokens.json (authtoken.DefaultPath),
  # peers.json (federation.DefaultPeersPath) and nodes.json (nodes.DefaultPath).
  "$CFG_WKS"
  # The hub is what loads plugins, so unlike on the node this directory is live.
  # Empty by default: see RUNBOOK.md §9 on why the always-on machine ships none.
  "$CFG_WKS/plugins"
  # Brain-owned stores. Dormant here (--brain-scope off means no brain), created
  # anyway so pointing a brain at this volume later is a flag, not a migration.
  "$CFG_WKS/library"
  "$CFG_WKS/layouts"
  "$CFG_WKS/sessions"
  "$CFG_WKS/logs"
  # os.UserConfigDir()/workspacer-hub — defaultPushDir, defaultJobsFile,
  # defaultLayoutFile in cmd/hub/main.go. On Linux os.UserConfigDir() reads
  # $XDG_CONFIG_HOME, which is why pinning XDG under $HOME relocates all four.
  "$CFG_HUB"
  # tailscaled's identity. The whole directory, not just tailscaled.state.
  "$WKS_DATA/tailscale"
)

bs_created=0
for d in "${bs_dirs[@]}"; do
  if [ ! -d "$d" ]; then
    mkdir -p "$d"
    bs_created=$((bs_created + 1))
  fi
done

# tailscaled.state holds the node key; the config dirs hold bearer credentials
# and, potentially, a cloud token that spends money. Owner-only, both.
chmod 700 "$WKS_DATA/tailscale"
chmod 700 "$CFG_HUB"

# ---------------------------------------------------------------------------
# 2. First boot vs every later boot (the directory layout).
# ---------------------------------------------------------------------------
bs_version_file="$WKS_DATA/state/layout-version"
if [ ! -f "$bs_version_file" ]; then
  bs_log "FIRST BOOT on this volume — created $bs_created directories, layout v$WKS_LAYOUT_VERSION."
  printf '%s\n' "$WKS_LAYOUT_VERSION" >"$bs_version_file"
else
  bs_have="$(cat "$bs_version_file" 2>/dev/null || echo 0)"
  if [ "$bs_have" != "$WKS_LAYOUT_VERSION" ]; then
    bs_log "volume was built by layout v$bs_have, this image is v$WKS_LAYOUT_VERSION — created $bs_created missing directories."
    printf '%s\n' "$WKS_LAYOUT_VERSION" >"$bs_version_file"
  elif [ "$bs_created" -gt 0 ]; then
    # Not first boot, yet directories were missing: something deleted them.
    # Worth a line in the boot log rather than silence.
    bs_log "populated volume (layout v$bs_have) but $bs_created directories were MISSING and have been recreated."
  else
    bs_log "populated volume (layout v$bs_have), all ${#bs_dirs[@]} directories present."
  fi
fi

# ---------------------------------------------------------------------------
# 3. THE STATE GUARD — first run vs state loss, per file.
# ---------------------------------------------------------------------------
# For each create-once file: is it here, is this the first time it has ever been
# expected, or has it VANISHED? Only the third is a problem, and only the third
# is indistinguishable from the first to the code that loads it.
#
# Evidence, strongest first:
#   1. $SEEN/<slug> — this file has existed on this volume before. A fact.
#   2. the statelost shape — its directory still holds entries other than it.
#      An inference, and the one internal/statelost makes; consulted so a volume
#      older than this marker scheme is still protected.
#
# Severity is per file and is argued, not uniform:
#   refuse  losing it makes the hub a DIFFERENT hub, or removes its only reason
#           to exist. A crash loop is the loudest signal this machine has.
#   warn    real, silent damage that a restart cannot make worse.
#   note    inconvenience; recorded so the boot log is complete.
bs_losses=0
bs_refusals=0

# bs_seen_slug turns a path relative to $WKS_HOME into a flat marker name.
# The leading dot is stripped so the markers are visible to a plain `ls` — an
# operator debugging a refusal should not have to know to pass -a.
bs_seen_slug() { printf '%s' "${1#"$WKS_HOME"/}" | tr '/' '_' | sed 's/^\.//'; }

# bs_present: exists AND is non-empty. A truncated credential is not a
# credential — internal/statelost's own tests treat that as loss, and
# loadOrCreateToken discards a whitespace-only remote-token.
bs_present() { [ -s "$1" ]; }

# --- the directory inference, and its two traps -----------------------------
#
# internal/statelost.Suspected asks: does the directory around the missing file
# still hold the rest of the state? Reproducing it here needs two corrections,
# and both were found by this script refusing to start on an empty volume.
#
# TRAP 1 — our own mkdirs are not evidence. Suspected counts ANY entry, including
# an empty subdirectory, and is right to: nothing pre-creates directories in the
# install it guards. Here §1 above mkdir -p's plugins/, library/, layouts/,
# sessions/ and logs/ inside the very directory being judged, on the FIRST boot.
# The unmodified rule then reports "somebody has run here" against a volume this
# script has only just created. So an EMPTY directory does not count: it is
# evidence that a mkdir ran, not that a hub did. A directory with anything in it
# — an installed plugin, a saved layout — is real state and counts normally.
#
# TRAP 2 — the guard must not become its own evidence. Minting the pairing
# credential writes a file into the directory that the NEXT guarded file is
# judged against, so nodes.json would be reported lost on a volume where nothing
# was ever lost. The evidence is therefore SNAPSHOTTED before any write, and
# every question is answered from the snapshot. Order-independent by construction.
bs_state_snapshot=""

# bs_snapshot_dir records dir's real-state entries as "<dir>/<name>" tokens.
bs_snapshot_dir() {
  local dir="$1" e
  [ -d "$dir" ] || return 0
  for e in "$dir"/* "$dir"/.[!.]*; do
    [ -e "$e" ] || continue
    if [ -d "$e" ] && [ -z "$(ls -A "$e" 2>/dev/null)" ]; then
      continue
    fi
    bs_state_snapshot="${bs_state_snapshot} ${dir}/$(basename "$e")"
  done
}

# bs_dir_had_other: did dir hold, at snapshot time, any real state that is not
# `name`. A directory that was never snapshotted holds nothing: nobody ran there.
bs_dir_had_other() {
  local dir="$1" name="$2" tok
  for tok in $bs_state_snapshot; do
    case "$tok" in
      "$dir"/*) [ "$tok" = "$dir/$name" ] || return 0 ;;
    esac
  done
  return 1
}

bs_snapshot_dir "$CFG_WKS"
bs_snapshot_dir "$CFG_HUB"
bs_snapshot_dir "$WKS_DATA/tailscale"

# bs_guard <severity> <path> <consequence>
# Returns 0 if the file is present, 1 if it is absent (first run OR loss).
# Records the marker on presence; reports and counts on loss.
#
# MARKER ONLY, no directory inference — and that is the third correction the
# statelost shape needs here. Suspected's inference is sound exactly when "the
# hub has run in this directory" implies "this file existed", and on the hub that
# is true of ONE file: remote-token, which every first boot creates
# unconditionally. It is false of every file below. tokens.json exists only if
# somebody minted a scoped token; layout.json only once a client saved a layout;
# jobs.json only once a job was created; nodes.json only on a hub that has a
# remote node. Judging those by their neighbours reports loss on a perfectly
# healthy hub that simply never had them — and a guard that cries wolf on every
# boot is a guard the operator turns off.
#
# The marker is the right evidence for all of them, because it records the fact
# rather than inferring it. What that gives up is the case where BOTH the file
# and $SEEN are destroyed while the config dir survives; the token, which keeps
# the inference below, is the one file where that gap is worth closing.
bs_guard() {
  local severity="$1" path="$2" consequence="$3"
  local marker; marker="$SEEN/$(bs_seen_slug "$path")"
  local name; name="$(basename "$path")"

  if bs_present "$path"; then
    [ -e "$marker" ] || : >"$marker"
    return 0
  fi

  local suspected=0
  [ -e "$marker" ] && suspected=1

  if [ "$suspected" = 0 ]; then
    bs_log "  $name: absent, and nothing on this volume says it ever existed — first run."
    return 1
  fi

  bs_losses=$((bs_losses + 1))
  if [ "$severity" = "refuse" ] && [ "$WKS_ALLOW_STATE_LOSS" != "1" ]; then
    bs_refusals=$((bs_refusals + 1))
    bs_log "STATE LOSS: $path is GONE, but this volume has held it before." >&2
    bs_log "  $consequence" >&2
    return 1
  fi
  if [ "$severity" = "note" ]; then
    bs_log "  $name: was present on this volume before and is now gone. $consequence"
  else
    bs_warn "STATE LOSS: $path is GONE, but this volume has held it before."
    bs_warn "  $consequence"
  fi
  return 1
}

bs_log "state guard: checking the create-once files"

# --- the pairing credential -------------------------------------------------
# THE ONE THAT MATTERS MOST. Re-minting is not a recovery, it is a new identity:
# it revokes every existing pairing at once and the process then prints a
# perfectly healthy ready banner. A phone that stopped working, a peer stuck on
# hub.peer.disconnected, and a hub that was never provisioned all look identical
# from outside. cmd/workspacer/token.go carries this argument at length; the hub
# binary never runs that code, so the rule is enforced here instead.
bs_token_file="$CFG_WKS/remote-token"
bs_token=""
bs_minted=0

bs_mint_token() {
  # 24 random bytes, base64url, no padding — byte-identical in shape to
  # loadOrCreateToken (crypto/rand + base64.RawURLEncoding) and to the desktop.
  head -c 24 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n'
}

if [ -n "${HUB_TOKEN:-}" ]; then
  # The operator declared the credential out of band (a Fly secret). That is the
  # recommended shape: the token then survives the volume, and rotating it is
  # `fly secrets set` rather than an edit to a file inside a running machine.
  if bs_present "$bs_token_file"; then
    bs_existing="$(tr -d '[:space:]' <"$bs_token_file")"
    if [ "$bs_existing" != "$HUB_TOKEN" ]; then
      if [ "$WKS_ALLOW_TOKEN_CHANGE" = "1" ]; then
        bs_warn "\$HUB_TOKEN differs from the persisted $bs_token_file and WKS_ALLOW_TOKEN_CHANGE=1 was set."
        bs_warn "  Adopting \$HUB_TOKEN. EVERY client, phone and federation peer paired against the old value is now refused."
        printf '%s' "$HUB_TOKEN" >"$bs_token_file"
        chmod 600 "$bs_token_file"
      else
        bs_refusals=$((bs_refusals + 1))
        bs_log "IDENTITY CONFLICT: \$HUB_TOKEN is set, and it is NOT the credential on this volume." >&2
        bs_log "  $bs_token_file holds a different value. Starting with \$HUB_TOKEN would silently refuse every" >&2
        bs_log "  client, phone and federation peer paired against the persisted one, while the hub looked healthy." >&2
        bs_log "  Either unset the secret (\`fly secrets unset HUB_TOKEN\`) to keep the pairing this volume already has," >&2
        bs_log "  or accept the new identity deliberately with WKS_ALLOW_TOKEN_CHANGE=1 and re-pair everything." >&2
      fi
    fi
  else
    # Mirror it onto the volume so the `workspacer` CLI on this machine, and
    # anything else that reads the persisted file, agree with what the hub serves.
    printf '%s' "$HUB_TOKEN" >"$bs_token_file"
    chmod 600 "$bs_token_file"
    bs_log "  remote-token: seeded from \$HUB_TOKEN (the recommended shape — the credential outlives the volume)."
  fi
  bs_token="$HUB_TOKEN"
  : >"$SEEN/$(bs_seen_slug "$bs_token_file")"
else
  # Hand-rolled rather than bs_guard, for two reasons the generic guard cannot
  # express. First, this is the ONE file where the statelost directory inference
  # is sound — every first boot creates a remote-token unconditionally, so
  # neighbours in the config dir really do imply it once existed — and it is
  # consulted alongside the marker so a wiped $SEEN cannot launder a loss into a
  # first run. Second, "refuse" and "mint" are the same decision here: an
  # operator who has already said WORKSPACER_ALLOW_NEW_TOKEN=1 must not also be
  # counted as a refusal, which is what asking the generic guard first did.
  bs_token_marker="$SEEN/$(bs_seen_slug "$bs_token_file")"
  bs_token_allow_new=0
  if [ "$WORKSPACER_ALLOW_NEW_TOKEN" = "1" ] || [ "$WKS_ALLOW_STATE_LOSS" = "1" ]; then
    bs_token_allow_new=1
  fi

  if bs_present "$bs_token_file"; then
    bs_token="$(tr -d '[:space:]' <"$bs_token_file")"
    [ -e "$bs_token_marker" ] || : >"$bs_token_marker"
  else
    bs_token_lost=0
    [ -e "$bs_token_marker" ] && bs_token_lost=1
    bs_dir_had_other "$CFG_WKS" "remote-token" && bs_token_lost=1

    if [ "$bs_token_lost" = 1 ] && [ "$bs_token_allow_new" != 1 ]; then
      bs_losses=$((bs_losses + 1))
      bs_refusals=$((bs_refusals + 1))
      bs_log "STATE LOSS: $bs_token_file is GONE, but this volume has held it before." >&2
      bs_log "  That file is the pairing credential — the bearer secret on /bus, /remote, /m and /app, and the" >&2
      bs_log "  value a federating peer presents to reach this hub. Minting a new one does not restore service, it" >&2
      bs_log "  REVOKES every existing pairing at once, and the hub then comes up looking perfectly healthy." >&2
      bs_log "  Restore it from a volume snapshot or your backup, or supply it with \`fly secrets set HUB_TOKEN=…\`." >&2
      bs_log "  If it is gone for good: re-pair every client and start once with WORKSPACER_ALLOW_NEW_TOKEN=1." >&2
    else
      [ "$bs_token_lost" = 1 ] && bs_losses=$((bs_losses + 1))
      [ "$bs_token_lost" = 0 ] &&
        bs_log "  remote-token: absent, and nothing on this volume says it ever existed — first run."
      bs_token="$(bs_mint_token)"
      printf '%s' "$bs_token" >"$bs_token_file"
      chmod 600 "$bs_token_file"
      : >"$bs_token_marker"
      bs_minted=1
      bs_token_was_lost="$bs_token_lost"
    fi
  fi
fi
bs_token_was_lost="${bs_token_was_lost:-0}"

# --- the node registry ------------------------------------------------------
# The quietest failure on this machine. nodes.LoadFile returns (nil, nil) for a
# missing file and startNodes then returns before registering anything, so a hub
# with no nodes.json has no nodes.list, no nodes.wake and no reconcile loop —
# and says so nowhere. That is precisely "healthy, and cannot do the one job this
# machine exists for". A corrupt file, by contrast, is already log.Fatalf.
#
# The marker is doing real work here: an operator who legitimately runs a hub
# with no remote nodes must not be refused on every boot, so absence is only a
# problem once this volume has actually held one.
bs_nodes_file="$CFG_WKS/nodes.json"
if bs_guard refuse "$bs_nodes_file" \
  "That file is the node registry. The hub treats a MISSING registry as \"no nodes\" — no error, no
  warning, and nodes.list / nodes.wake are never registered — so this hub would come up healthy and
  be unable to wake anything, which is the only reason it is always on.
  Restore it from a volume snapshot, or recreate it (RUNBOOK.md §8) and restart."; then
  # Not ours to write, but the mode is ours to repair: the file may carry an
  # inline Fly API token, and the hub only warns about that after it has already
  # been readable. FileLooksExposed will confirm from the other side.
  bs_mode="$(stat -c '%a' "$bs_nodes_file" 2>/dev/null || echo 600)"
  if [ "$bs_mode" != "600" ]; then
    chmod 600 "$bs_nodes_file"
    bs_log "  nodes.json was mode $bs_mode — tightened to 600 (it may hold a cloud token that spends money)."
  fi
fi

# --- Web Push -------------------------------------------------------------
# WARN, not refuse, and the asymmetry is deliberate: push.New's own reasoning is
# that stopping the bus, the sessions and federation because a notifications
# keypair went missing is a far larger outage than the one being reported. The
# hub itself now logs a STATE LOSS line and drops the dead subscriptions. This
# check runs BEFORE the hub starts so the warning appears in boot order, on the
# volume, next to the mount that failed.
bs_guard warn "$CFG_HUB/vapid.json" \
  "That is the generate-once Web Push application-server keypair. The hub will generate a NEW one and
  drop every stored subscription: each device stops receiving agent-needs-you alerts and every one of
  them still reports itself subscribed. If the old file is recoverable, restore it and restart NOW —
  once devices re-subscribe against the new key the old one is worthless." || true

bs_guard note "$CFG_HUB/push-subscriptions.json" \
  "Each device re-subscribes on its next visit to /m." || true

# --- the rest of the hub's persistent state ---------------------------------
bs_guard warn "$CFG_WKS/tokens.json" \
  "Capability-scoped tokens (minted by \`workspacer token create\`). authtoken.Load treats a missing
  file as an EMPTY store, not an error, so every scoped token is now denied — fail-closed, correct,
  and still a wake-blocker for any node whose brain presents one." || true

bs_guard warn "$CFG_WKS/peers.json" \
  "The federation peer list, read ONCE at hub start. Every peer hub is now invisible to every client." || true

bs_guard warn "$CFG_HUB/jobs.json" \
  "Every recurring and one-off hub job. Nothing fires and nothing errors, because from the hub's
  point of view you simply have no jobs." || true

bs_guard warn "$CFG_WKS/config.yaml" \
  "Settings. Nothing on THIS machine reads it (the hub binary does not; the brain does, and
  --brain-scope off means there is no brain here) — but a brain pointed at this volume later, or the
  \`workspacer\` CLI, would silently see defaults." || true

bs_guard note "$CFG_HUB/layout.json" \
  "The shared workspace layout resets to empty." || true

bs_guard note "$CFG_HUB/jobs-history.json" \
  "Job run history resets." || true

# --- the tailnet identity ---------------------------------------------------
# Not a config file, but the same shape: regenerated silently, and the node's
# peers.json / the brain's --hub URL point at a MagicDNS name that now resolves
# to a different device.
bs_guard warn "$WKS_DATA/tailscale/tailscaled.state" \
  "The tailnet identity. tailscaled will register as a NEW device with a NEW address, so the name in
  the node's HUB_BUS_URL resolves elsewhere and the node can never attach. Check the Tailscale admin
  console for a duplicate device before doing anything else." || true

if [ "$bs_losses" -eq 0 ]; then
  bs_log "state guard: no losses detected"
fi

# ---------------------------------------------------------------------------
# 4. Ownership.
# ---------------------------------------------------------------------------
# A Fly volume mounts root-owned, and every state file here is 0600 or 0700, so
# a uid mismatch after a fresh volume or a snapshot restore is fatal rather than
# degraded: push.New returns an error the hub treats as fatal, and the token file
# cannot be written at all.
#
# This volume is small (config files and boot logs, not repos and module caches),
# so the node's marker-gated deep-chown optimisation would be optimising away
# something that already costs milliseconds. The marker is still written, because
# the boot log saying which pass ran is worth more than the pass costs.
bs_owner_marker="$WKS_DATA/state/owner-uid"
bs_want="$WKS_UID:$WKS_GID"
if [ "$WKS_SKIP_CHOWN" = "1" ]; then
  bs_log "ownership reconciliation skipped (WKS_SKIP_CHOWN=1)"
elif [ "$(cat "$bs_owner_marker" 2>/dev/null || echo none)" = "$bs_want" ]; then
  chown -R "$bs_want" "$WKS_DATA" 2>/dev/null || true
  bs_log "ownership marker matches $bs_want — reconciled $WKS_DATA"
else
  bs_log "ownership marker missing or stale — chowning $WKS_DATA to $bs_want (first boot, or a restored/rebuilt volume)."
  chown -R "$bs_want" "$WKS_DATA"
  printf '%s\n' "$bs_want" >"$bs_owner_marker"
fi

# ---------------------------------------------------------------------------
# 5. Shell skeleton.
# ---------------------------------------------------------------------------
# Split deliberately: .wks-env is GENERATED and rewritten by the entrypoint every
# boot; .bashrc/.profile are the operator's and are only ever seeded or appended
# to. An env change then ships with the image without clobbering anything a human
# put on the volume.
for rc in .bashrc .profile; do
  f="$WKS_HOME/$rc"
  [ -e "$f" ] || : >"$f"
  if ! grep -qF '.wks-env' "$f" 2>/dev/null; then
    {
      echo ''
      echo '# workspacer fly hub: the same environment the hub runs with.'
      echo '# Generated by the entrypoint on every boot; edit the image, not this line.'
      # shellcheck disable=SC2016  # written literally into .bashrc, expanded there
      echo '[ -f "$HOME/.wks-env" ] && . "$HOME/.wks-env"'
    } >>"$f"
    bs_log "seeded $rc with the .wks-env hook"
  fi
done
[ -e "$WKS_HOME/.bash_history" ] || : >"$WKS_HOME/.bash_history"
chown "$bs_want" "$WKS_HOME/.bashrc" "$WKS_HOME/.profile" "$WKS_HOME/.bash_history" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 6. Self-check: assert the symlink rule.
# ---------------------------------------------------------------------------
# Cheap insurance against a future edit reintroducing a file symlink, which a
# tmp+rename write destroys silently.
if bs_bad_links="$(find "$WKS_HOME" -maxdepth 3 -type l 2>/dev/null)" && [ -n "$bs_bad_links" ]; then
  bs_warn "symlinks found under \$HOME — a tmp+rename write will silently replace any that point at a FILE:"
  while IFS= read -r bs_link; do bs_warn "  $bs_link"; done <<<"$bs_bad_links"
fi

# ---------------------------------------------------------------------------
# 7. Verdict.
# ---------------------------------------------------------------------------
if [ "$bs_refusals" -gt 0 ]; then
  bs_log "REFUSING TO START: $bs_refusals unrecoverable state problem(s) above." >&2
  bs_log "  A hub that starts anyway does not fail — it becomes a DIFFERENT hub, and every client it" >&2
  bs_log "  turns away still sees a healthy banner. Fix the cause, or say so deliberately:" >&2
  bs_log "    fly secrets set WKS_ALLOW_STATE_LOSS=1   (accept ALL of the above, once)" >&2
  bs_log "    fly secrets set WORKSPACER_ALLOW_NEW_TOKEN=1  (accept a new pairing credential only)" >&2
  exit 2
fi

if [ "$bs_minted" = 1 ]; then
  # The distinction the whole file is about, said out loud in the boot log: a
  # first run and a recovery both produce a working hub with a new credential,
  # and only one of them means "everything you paired is now refused".
  if [ "$bs_token_was_lost" = 0 ]; then
    bs_log "FIRST RUN: minted a new pairing credential at $bs_token_file"
  else
    bs_log "NEW PAIRING CREDENTIAL minted at $bs_token_file (state loss was accepted explicitly)"
  fi
  bs_log "  HUB_TOKEN=$bs_token"
  bs_log "  Pair every client with it, then consider moving it into \`fly secrets set HUB_TOKEN=…\` so it"
  bs_log "  outlives this volume. It is now in this boot log and in \`fly logs\`; rotate if that matters."
fi

# The entrypoint reads the credential back from the file rather than from here:
# a value on stdout would land in the boot log on every single boot, not just
# the one that minted it.
bs_log "ready: HOME=$WKS_HOME on $WKS_DATA"
