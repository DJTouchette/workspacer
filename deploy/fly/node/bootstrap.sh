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
# CONTRACT
# ---------------------------------------------------------------------------
# Idempotent. Safe on an EMPTY volume (first boot) and on a POPULATED one
# (every later boot). Creates what is missing, never overwrites what exists.
# Pure function of the environment below, so it can be exercised against a
# temp directory standing in for /data — see test-bootstrap.sh.
#
#   WKS_DATA            volume root                       (default /data)
#   WKS_HOME            $HOME on the volume               (default $WKS_DATA/home)
#   WKS_UID / WKS_GID   uid:gid that must own the volume  (default 10001:10001)
#   WKS_REQUIRE_MOUNT   1 = refuse to run unless $WKS_DATA is a real mountpoint
#   WKS_SKIP_CHOWN      1 = skip ownership reconciliation (local tests, non-root)
#
set -euo pipefail

WKS_DATA="${WKS_DATA:-/data}"
WKS_HOME="${WKS_HOME:-${WKS_DATA}/home}"
WKS_UID="${WKS_UID:-10001}"
WKS_GID="${WKS_GID:-10001}"
WKS_REQUIRE_MOUNT="${WKS_REQUIRE_MOUNT:-1}"
WKS_SKIP_CHOWN="${WKS_SKIP_CHOWN:-0}"

# Bumped when the layout below changes in a way an existing volume must be
# told about. Recorded on the volume so a later boot can tell "first boot" from
# "boot against a volume built by an older image".
WKS_LAYOUT_VERSION=1

bs_log() { printf '%s bootstrap: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*"; }
bs_die() { bs_log "FATAL: $*" >&2; exit 1; }

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
# 3. Shell skeleton.
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
# 4. Ownership.
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
# 5. Self-check: assert the rule.
# ---------------------------------------------------------------------------
# Cheap insurance against a future edit reintroducing a file symlink. A symlink
# to a FILE anywhere under $HOME is the failure this whole design avoids.
if bs_bad_links="$(find "$WKS_HOME" -maxdepth 3 -type l 2>/dev/null)" && [ -n "$bs_bad_links" ]; then
  bs_log "WARNING: symlinks found under \$HOME — a tmp+rename write will silently replace any that point at a FILE:"
  while IFS= read -r bs_link; do bs_log "  $bs_link"; done <<<"$bs_bad_links"
fi

bs_log "ready: HOME=$WKS_HOME on $WKS_DATA"
