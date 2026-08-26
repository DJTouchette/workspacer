#!/usr/bin/env bash
#
# bootstrap.sh — prepare the Fly volume so a woken machine finds every
# credential, cache and database exactly where it left them.
#
# ---------------------------------------------------------------------------
# THE RULE THIS FILE EXISTS TO ENFORCE
# ---------------------------------------------------------------------------
# Persist DIRECTORIES, never individual files.
#
# Every atomic writer in this stack writes a sibling temp file and then
# rename(2)s it over the target. rename() replaces the *directory entry*, so a
# FILE symlink is destroyed and replaced by a regular file on the ephemeral
# rootfs. It works perfectly for the rest of that boot and the data is gone on
# the next wake — the exact silent, intermittent failure this deployment must
# not have. (Verified locally: see test-bootstrap.sh, case "symlink rule".)
#
# So this script creates NO symlinks at all. Instead $HOME itself lives on the
# volume ($WKS_DATA/home). Everything dotfile-shaped — ~/.claude, ~/.claude.json,
# ~/.claudemon, ~/.config/workspacer, ~/.workspacer, ~/.ssh, ~/.gitconfig,
# ~/.codex — is then persistent by construction, with no link to destroy.
#
# That also keeps the config.yaml twins in agreement: the Go half reads
# $XDG_CONFIG_HOME/workspacer and the TS half hardcodes ~/.config/workspacer.
# With XDG_CONFIG_HOME pinned to $HOME/.config and $HOME on the volume, both
# resolve to the same persistent directory. Relocating XDG_CONFIG_HOME to a
# separate /data/config would have split them.
#
# ---------------------------------------------------------------------------
# THE SECOND JOB: FIRST RUN vs STATE LOSS
# ---------------------------------------------------------------------------
# Preparing the volume was this file's only job for a while, and that was the
# wrong shape, because of one sentence:
#
#   *** EVERYTHING IRREPLACEABLE IN THIS DESIGN IS ON THIS ONE VOLUME. ***
#
# The always-on hub holds config and a pairing credential. This machine holds
# the Claude OAuth session, the SSH key, ~/.claude.json's folder-trust map and
# the tailnet identity, and every one of them is loaded by code shaped "read it;
# if it is not there, carry on". Take ~/.claude/.credentials.json off this volume
# by any means, a stray rm, a botched restore that brought back some of the tree,
# a truncation, and this node boots green, registers its 67 capabilities, reports
# `available`, and every session dispatched to it parks on a login prompt no
# headless machine can answer. Nothing errors. Nothing logs.
#
# So the hub's guard is ported here rather than reinvented: a per-file marker
# under $WKS_DATA/state/seen records the FACT that a file has existed on this
# volume before, and absence is only a problem once that marker exists. A first
# boot, where none of these files exists yet and the operator has not done the
# interactive logins, stays quiet. See bs_guard.
#
# WHAT THIS DOES NOT CATCH, said here rather than discovered later, because the
# hub's bootstrap states its own version of this gap and these two must be read
# the same way:
#
#   *** A WHOLE-VOLUME ROLLBACK IS INVISIBLE TO THIS GUARD. ***
#
# The evidence lives ON the volume it is evidence about. Restore a snapshot taken
# before the interactive logins and $SEEN comes back exactly as it was then, with
# no markers in it, so a genuinely lost credential reads as a first run and this
# file says nothing. That is the failure mode described above, arriving by the
# one route the marker cannot see, and no amount of on-volume bookkeeping closes
# it: the evidence would have to outlive the volume.
#
# What the marker DOES catch is everything that removes a file while leaving the
# volume otherwise intact, which is every other way this has gone wrong: a stray
# rm, a partial or interrupted restore, a truncation to zero bytes (bs_present
# checks size, not existence), a wipe of one subtree. Those are the common cases.
# The rollback is the rare one, and the answer to it is the snapshot-retention
# discipline in RUNBOOK.md plus §8 check 14, which is there so that an operator
# who has just restored a volume looks at `ls /data/state/seen` and sees an empty
# directory on a node that should have four markers.
#
# ---------------------------------------------------------------------------
# CONTRACT
# ---------------------------------------------------------------------------
# Idempotent. Safe on an EMPTY volume (first boot), a POPULATED one (every later
# boot), and a DAMAGED one (reports and, where it must, refuses). Creates what is
# missing, never overwrites what exists. Pure function of the environment below,
# so it can be exercised against a temp directory standing in for /data, see
# test-bootstrap.sh.
#
#   WKS_DATA            volume root                       (default /data)
#   WKS_HOME            $HOME on the volume               (default $WKS_DATA/home)
#   WKS_UID / WKS_GID   uid:gid that must own the volume  (default 10001:10001)
#   WKS_REQUIRE_MOUNT   1 = refuse to run unless $WKS_DATA is a real mountpoint
#   WKS_SKIP_CHOWN      1 = skip ownership reconciliation (local tests, non-root)
#   WKS_ALLOW_STATE_LOSS=1  downgrade every refusal below to a warning
#
# Exit 0 = the volume is ready and the node may start.
# Exit non-zero = do NOT start. The message says which file and why.
#
set -euo pipefail

WKS_DATA="${WKS_DATA:-/data}"
WKS_HOME="${WKS_HOME:-${WKS_DATA}/home}"
WKS_UID="${WKS_UID:-10001}"
WKS_GID="${WKS_GID:-10001}"
WKS_REQUIRE_MOUNT="${WKS_REQUIRE_MOUNT:-1}"
WKS_SKIP_CHOWN="${WKS_SKIP_CHOWN:-0}"
WKS_ALLOW_STATE_LOSS="${WKS_ALLOW_STATE_LOSS:-0}"

SEEN="$WKS_DATA/state/seen"              # one marker per create-once file ever observed

# Bumped when the layout below changes in a way an existing volume must be
# told about. Recorded on the volume so a later boot can tell "first boot" from
# "boot against a volume built by an older image".
WKS_LAYOUT_VERSION=1

bs_log()  { printf '%s bootstrap: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
bs_warn() { bs_log "WARNING: $*" >&2; }
bs_die()  { bs_log "FATAL: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 0. Refuse to run without the volume.
# ---------------------------------------------------------------------------
# Running on the ephemeral rootfs is the worst outcome available: tailscaled
# mints a new node key (new tailnet IP), the brain's config.yaml is silently
# reseeded with defaults, claudemon builds an empty schema, and the machine
# comes up looking healthy while being a different machine. Fail loudly instead.
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
# Sourced from the persistence scout's corrected /data list
# (.workspacer/reports/2026-08-24-fly-node-scout-persistence.md §3), rewritten
# for HOME-on-volume. RUNBOOK.md carries the scout-path -> here mapping.
bs_dirs=(
  # bookkeeping + the boot log, which must exist before anything else runs
  "$WKS_DATA/logs"
  "$WKS_DATA/state"
  "$SEEN"

  # $HOME. Persistent by construction; no symlinks anywhere below this point.
  "$WKS_HOME"
  "$WKS_HOME/.claude"                    # OAuth, projects/ (the cost ledger), settings.json, accounts/, plugins/
  "$WKS_HOME/.claude/projects"
  "$WKS_HOME/.codex"                     # codex OAuth + config.toml + skills, if the codex provider is used
  "$WKS_HOME/.config"                    # == $XDG_CONFIG_HOME
  "$WKS_HOME/.config/workspacer"         # config.yaml, tokens.json, remote-token, peers.json, claude-profiles.json
  "$WKS_HOME/.config/workspacer/plugins" # only loaded if --plugins-dir is passed; persisted regardless
  "$WKS_HOME/.config/workspacer/library"
  "$WKS_HOME/.config/workspacer/layouts"
  "$WKS_HOME/.config/workspacer/sessions"
  "$WKS_HOME/.config/workspacer/logs"
  "$WKS_HOME/.config/workspacer-hub"     # vapid.json, push-subscriptions.json, jobs.json, layout.json
  "$WKS_HOME/.config/git"                # git's XDG global config (fsguard reads this one)
  "$WKS_HOME/.config/gh"                 # gh CLI auth (hosts.yml) — not read by workspacer, but by git-over-gh
  "$WKS_HOME/.local/share"               # == $XDG_DATA_HOME
  "$WKS_HOME/.local/share/claudemon"     # state.db — passed explicitly via --db-path, see entrypoint.sh
  "$WKS_HOME/.local/state"               # == $XDG_STATE_HOME
  "$WKS_HOME/.workspacer"                # model-rates.json, scripts/, brief.md, claude-settings.json
  "$WKS_HOME/.workspacer/scripts"
  "$WKS_HOME/.workspacer/handoffs"
  "$WKS_HOME/.workspacer/codex-threads"
  "$WKS_HOME/.workspacer/worktrees"      # defaultWorktreeRoot(); must live or die with repos/ below

  # work + caches
  "$WKS_DATA/repos"
  "$WKS_DATA/go/pkg/mod"
  "$WKS_DATA/go/cache"
  "$WKS_DATA/bundle"
  "$WKS_DATA/bun"
  "$WKS_DATA/npm"
  "$WKS_DATA/cache/xdg"

  # tailscaled's identity. The whole directory, not just tailscaled.state —
  # tailscaled keeps other state beside it.
  "$WKS_DATA/tailscale"
)

bs_created=0
for d in "${bs_dirs[@]}"; do
  if [ ! -d "$d" ]; then
    mkdir -p "$d"
    bs_created=$((bs_created + 1))
  fi
done

# 0700 or ssh refuses the keys outright.
mkdir -p "$WKS_HOME/.ssh"
chmod 700 "$WKS_HOME/.ssh"
# tailscaled.state holds the node key. Owner-only.
chmod 700 "$WKS_DATA/tailscale"

# ---------------------------------------------------------------------------
# 2. First boot vs every later boot.
# ---------------------------------------------------------------------------
bs_version_file="$WKS_DATA/state/layout-version"
if [ ! -f "$bs_version_file" ]; then
  bs_log "FIRST BOOT on this volume — created $bs_created directories, layout v$WKS_LAYOUT_VERSION."
  bs_log "  Claude OAuth, git/ssh credentials and the tailnet identity are NOT here yet."
  bs_log "  See RUNBOOK.md step 6 for the one-time interactive logins."
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
    bs_log "populated volume (layout v$bs_have), all $((${#bs_dirs[@]})) directories present."
  fi
fi

# ---------------------------------------------------------------------------
# 3. THE STATE GUARD, first run vs state loss, per file.
# ---------------------------------------------------------------------------
# Ported from deploy/fly/hub/bootstrap.sh, deliberately as the SAME mechanism
# rather than a second one: the hub's markers and this one are read the same way
# by anyone debugging either machine.
#
# The evidence is a per-file marker under $SEEN. "This file has existed on this
# volume before" is a FACT. The alternative, internal/statelost's "the directory
# around it still holds the rest of the state", is an INFERENCE, and it is
# unsound for every file below: ~/.claude fills up with projects/ and
# settings.json on the first boot whether or not anyone has logged in, so its
# neighbours say nothing about whether .credentials.json ever existed. The marker
# records the fact instead of guessing at it.
#
# Severity is per file and is argued, not uniform:
#   refuse  the node cannot do the one thing it exists for, and would not say so.
#   warn    real, silent damage that starting anyway does not make worse.
#   note    inconvenience; recorded so the boot log is complete.
#
# REFUSING ON THIS MACHINE COSTS MORE THAN ON THE HUB, and that is why exactly
# one file gets it. The hub restarts `always` and is reachable; this node's Fly
# restart policy is on-failure, so a refusal means three quick retries and then
# `stopped`, and `fly ssh console` needs a machine that is running. A refusal
# here is close to a lockout, recoverable only by starting once with
# WKS_ALLOW_STATE_LOSS=1. It is worth that for the credential and for nothing
# else. (The refusal does at least reach `fly logs` now: the entrypoint replays
# the previous boot log to stdout on the next attempt.)
bs_losses=0
bs_refusals=0

# bs_seen_slug turns a guarded path into a flat marker name. The leading dot is
# stripped so the markers are visible to a plain `ls`: an operator debugging a
# refusal should not have to know to pass -a.
#
# BOTH prefixes are stripped, $WKS_HOME first because it is the longer one. The
# marker name must not depend on where the volume happens to be mounted: a slug
# carrying the mount path orphans every marker the moment $WKS_DATA moves, and an
# orphaned marker fails OPEN: a real loss then reads as a first run, which is
# the one direction this guard must never be wrong in. Identical in both
# bootstraps; keep them that way.
bs_seen_slug() {
  local p="${1#"$WKS_HOME"/}"
  p="${p#"$WKS_DATA"/}"
  printf '%s' "$p" | tr '/' '_' | sed 's/^\.//'
}

# bs_present: exists AND is non-empty. A truncated credential is not a
# credential, internal/statelost's own tests treat that as loss.
bs_present() { [ -s "$1" ]; }

# bs_guard <severity> <path> <consequence>
# Returns 0 if the file is present, 1 if it is absent (first run OR loss).
# Records the marker on presence; reports and counts on loss.
bs_guard() {
  local severity="$1" path="$2" consequence="$3"
  local marker; marker="$SEEN/$(bs_seen_slug "$path")"
  local name; name="$(basename "$path")"

  if bs_present "$path"; then
    [ -e "$marker" ] || : >"$marker"
    return 0
  fi

  if [ ! -e "$marker" ]; then
    bs_log "  $name: absent, and nothing on this volume says it ever existed. First run."
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

# --- the Claude OAuth session ----------------------------------------------
# THE ONE THAT MATTERS MOST, and the only refusal on this machine. Claude Code
# with no credential does not fail: it prompts, and a headless PTY has nobody to
# answer. The node boots green, the brain registers 67 capabilities, nodes.list
# says `available`, and every dispatched agent sits at a login screen forever.
# There is no error anywhere in that sequence, on either machine, which is
# exactly the class of failure this deployment refuses to have.
#
# Absent on a genuine first boot: the interactive login is a one-time step the
# operator does over `fly ssh console` (RUNBOOK.md §7a), so the marker is what
# separates "not logged in yet" from "the login was taken away".
bs_guard refuse "$WKS_HOME/.claude/.credentials.json" \
  "That is the Claude Code OAuth session: the credential every agent on this node runs as. Claude
  Code does not error without it, it PROMPTS, and nothing headless can answer a prompt: the node
  would come up healthy and every dispatched session would hang forever with no error on either
  machine. Restore it from a volume snapshot, or log in again (RUNBOOK.md §7a), which needs a
  shell, so start once with \`fly secrets set WKS_ALLOW_STATE_LOSS=1\`, log in, then unset it." || true

# --- the folder-trust map ---------------------------------------------------
# WARN, not refuse. Same silent-hang shape as the credential, but this file is
# cheap to recreate and a refusal here would double the lockout risk above for
# something a single heredoc fixes.
bs_guard warn "$WKS_HOME/.claude.json" \
  "That holds hasCompletedOnboarding and the per-directory folder-trust map. Without it Claude Code
  asks whether it trusts /data/repos/… on first use, and a headless session has nobody to answer:
  the symptom is a spawned worker that looks alive and does nothing. Recreate it per RUNBOOK.md §7." || true

# --- the git identity -------------------------------------------------------
bs_guard warn "$WKS_HOME/.ssh/id_ed25519" \
  "The node's SSH key. Every push, and every clone over ssh, now fails, visibly, at least, unlike
  the two above. Recreate it with ssh-keygen and re-add the public half (RUNBOOK.md §7b)." || true

# --- the tailnet identity ---------------------------------------------------
# WARN and not refuse, matching the hub's call on the same file: the brain DIALS
# OUT to $HUB_BUS_URL, so a new tailnet identity does not stop the node attaching.
# What it costs is the stable inbound name and address, and it leaves a duplicate
# device behind.
bs_guard warn "$WKS_DATA/tailscale/tailscaled.state" \
  "The tailnet identity. tailscaled will register as a NEW device with a NEW address, so this node
  gets a suffixed MagicDNS name and anything that reaches it BY name or IP, your ssh, the runbook's
  recorded ipv4, points at a device that no longer exists. Check the Tailscale admin console for a
  duplicate device. The brain dials out, so the hub attach itself still works." || true

# --- settings ---------------------------------------------------------------
# The brain says this too, from the other side, and says it better because it can
# list what reverted. Saying it HERE as well is not duplication: this line is in
# boot order, before anything has written over the seed, and it is on the volume.
bs_guard warn "$WKS_HOME/.config/workspacer/config.yaml" \
  "Settings. The brain reseeds factory defaults and runs on them: projects, agents.binaries (so
  provider resolution silently falls back to PATH), transport, supervisor.fullAccess, budgets and
  keybindings are all back to their shipped values. Restore it and restart BEFORE anything saves." || true

# --- session history --------------------------------------------------------
bs_guard note "$WKS_HOME/.local/share/claudemon/state.db" \
  "Every stopped-but-resumable session on this node is gone. Transcripts under ~/.claude/projects
  survive; the daemon's index of them does not." || true

if [ "$bs_losses" -eq 0 ]; then
  bs_log "state guard: no losses detected"
fi

# ---------------------------------------------------------------------------
# 4. Shell skeleton.
# ---------------------------------------------------------------------------
# Split deliberately: .wks-env is GENERATED and rewritten by the entrypoint on
# every boot; .bashrc/.profile are the operator's and are only ever seeded or
# appended to. That way an env change ships with the image without clobbering
# anything a human put on the volume.
for rc in .bashrc .profile; do
  f="$WKS_HOME/$rc"
  [ -e "$f" ] || : >"$f"
  if ! grep -qF '.wks-env' "$f" 2>/dev/null; then
    {
      echo ''
      echo '# workspacer fly node: the same environment the daemons run with.'
      echo '# Generated by the entrypoint on every boot; edit the image, not this line.'
      # shellcheck disable=SC2016  # written literally into .bashrc, expanded there
      echo '[ -f "$HOME/.wks-env" ] && . "$HOME/.wks-env"'
    } >>"$f"
    bs_log "seeded $rc with the .wks-env hook"
  fi
done
[ -e "$WKS_HOME/.bash_history" ] || : >"$WKS_HOME/.bash_history"

# ---------------------------------------------------------------------------
# 5. Ownership.
# ---------------------------------------------------------------------------
# A Fly volume mounts root-owned. Every state file this stack writes is 0600 or
# 0700, so a uid mismatch after a fresh volume or a snapshot restore is fatal,
# not degraded. But `chown -R` across a 30GB volume with a populated Go module
# cache is minutes of wake latency, and the wake budget is ~15s. So: deep chown
# only when a marker says the ownership is not already right.
bs_owner_marker="$WKS_DATA/state/owner-uid"
bs_want="$WKS_UID:$WKS_GID"
if [ "$WKS_SKIP_CHOWN" = "1" ]; then
  bs_log "ownership reconciliation skipped (WKS_SKIP_CHOWN=1)"
elif [ "$(cat "$bs_owner_marker" 2>/dev/null || echo none)" = "$bs_want" ]; then
  # Cheap pass: only the directories this run created can be wrong.
  chown "$bs_want" "${bs_dirs[@]}" "$WKS_HOME/.ssh" 2>/dev/null || true
  bs_log "ownership marker matches $bs_want — skipped the deep chown"
else
  bs_log "ownership marker missing or stale — deep chown of $WKS_DATA to $bs_want."
  bs_log "  This is a first boot or a restored/rebuilt volume. On a large volume it can take minutes; it happens once."
  chown -R "$bs_want" "$WKS_DATA"
  printf '%s\n' "$bs_want" >"$bs_owner_marker"
  bs_log "deep chown complete, marker written"
fi

# ---------------------------------------------------------------------------
# 6. Self-check: assert the rule.
# ---------------------------------------------------------------------------
# Cheap insurance against a future edit reintroducing a file symlink. A symlink
# to a FILE anywhere under $HOME is the failure this whole design avoids.
if bs_bad_links="$(find "$WKS_HOME" -maxdepth 3 -type l 2>/dev/null)" && [ -n "$bs_bad_links" ]; then
  bs_log "WARNING: symlinks found under \$HOME — a tmp+rename write will silently replace any that point at a FILE:"
  while IFS= read -r bs_link; do bs_log "  $bs_link"; done <<<"$bs_bad_links"
fi

# ---------------------------------------------------------------------------
# 7. Verdict.
# ---------------------------------------------------------------------------
if [ "$bs_refusals" -gt 0 ]; then
  bs_log "REFUSING TO START: $bs_refusals unrecoverable state problem(s) above." >&2
  bs_log "  A node that starts anyway does not fail: it reports itself available and then hangs every" >&2
  bs_log "  session dispatched to it, with no error on this machine or the hub. Fix the cause, or say" >&2
  bs_log "  so deliberately and start once with:" >&2
  bs_log "    fly secrets set WKS_ALLOW_STATE_LOSS=1   (accept ALL of the above)" >&2
  bs_log "  That is also how you get a shell back on this machine to redo the interactive logins." >&2
  exit 2
fi

bs_log "ready: HOME=$WKS_HOME on $WKS_DATA"
