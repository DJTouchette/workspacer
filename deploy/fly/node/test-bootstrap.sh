#!/usr/bin/env bash
#
# test-bootstrap.sh — exercise bootstrap.sh against a temp directory standing
# in for /data, covering BOTH the empty-volume (first boot) and populated-volume
# (every later boot) cases.
#
# This exists because the interesting failures in this deployment are not
# "it crashed" — they are "it came up looking healthy against the wrong state".
# A start script symlinking into an empty volume behaves differently from one
# symlinking into a populated one, and that difference is a classic source of
# intermittent failure. So the difference is asserted here, on a laptop, rather
# than discovered on a machine that only wakes up once a fortnight.
#
#   ./test-bootstrap.sh          run everything
#
# No root, no Docker, no Fly. Runs in about a second.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$HERE/bootstrap.sh"

pass=0
fail=0

section() { printf '\n\033[1m== %s\033[0m\n' "$1"; }
ok()   { pass=$((pass+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }

assert()          { if [ "$1" = 0 ]; then ok "$2"; else bad "$2"; fi; }
assert_dir()      { if [ -d "$1" ]; then ok "dir exists: ${1#"$TMP"}"; else bad "dir MISSING: ${1#"$TMP"}"; fi; }
assert_file()     { if [ -f "$1" ]; then ok "file exists: ${1#"$TMP"}"; else bad "file MISSING: ${1#"$TMP"}"; fi; }
assert_mode()     { local m; m="$(stat -c '%a' "$1")"; if [ "$m" = "$2" ]; then ok "mode $2 on ${1#"$TMP"}"; else bad "mode on ${1#"$TMP"} is $m, want $2"; fi; }
assert_content()  { local got; got="$(cat "$1" 2>/dev/null)"; if [ "$got" = "$2" ]; then ok "$3"; else bad "$3 (got: '$got')"; fi; }
assert_grep()     { if grep -qF "$2" <<<"$1"; then ok "$3"; else bad "$3"; fi; }
assert_not_grep() { if grep -qF "$2" <<<"$1"; then bad "$3"; else ok "$3"; fi; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/wks-bootstrap-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# The volume is not a real mountpoint here, so every run passes
# WKS_REQUIRE_MOUNT=0 except the case that specifically tests the refusal.
run_bootstrap() {
  local data="$1"; shift
  env WKS_DATA="$data" WKS_HOME="$data/home" \
      WKS_UID="$(id -u)" WKS_GID="$(id -g)" \
      WKS_REQUIRE_MOUNT=0 "$@" bash "$BOOTSTRAP" 2>&1
}

# ===========================================================================
section "refuses to run without the volume"
# ===========================================================================
# Running on the ephemeral rootfs is the worst available outcome: a new tailnet
# identity, a silently reseeded config.yaml, an empty session database — and a
# machine that looks perfectly healthy while being a different machine.

out="$(env WKS_DATA="$TMP/does-not-exist" WKS_REQUIRE_MOUNT=0 bash "$BOOTSTRAP" 2>&1)"; rc=$?
assert "$([ $rc -ne 0 ] && echo 0 || echo 1)" "nonzero exit when \$WKS_DATA does not exist (rc=$rc)"
assert_grep "$out" "does not exist" "says the volume is not mounted"

mkdir -p "$TMP/not-a-mount"
out="$(env WKS_DATA="$TMP/not-a-mount" WKS_REQUIRE_MOUNT=1 bash "$BOOTSTRAP" 2>&1)"; rc=$?
assert "$([ $rc -ne 0 ] && echo 0 || echo 1)" "nonzero exit when \$WKS_DATA exists but is not a mountpoint (rc=$rc)"
assert_grep "$out" "not a mountpoint" "distinguishes 'not mounted' from 'missing'"

# ===========================================================================
section "FIRST BOOT — empty volume"
# ===========================================================================
D="$TMP/empty"; H="$D/home"
mkdir -p "$D"
out="$(run_bootstrap "$D" WKS_SKIP_CHOWN=0)"; rc=$?

assert "$rc" "exits 0 on an empty volume"
assert_grep "$out" "FIRST BOOT" "announces the first boot"
assert_grep "$out" "are NOT here yet" "warns that the interactive logins have not happened"

# The scout's corrected /data list, every entry, in its HOME-on-volume home.
assert_dir "$H/.claude"                        # Claude OAuth + projects/ (the cost ledger)
assert_dir "$H/.claude/projects"
assert_dir "$H/.codex"                         # codex OAuth, if the codex provider is used
assert_dir "$H/.config/workspacer"             # config.yaml, tokens.json, remote-token, peers.json
assert_dir "$H/.config/workspacer/plugins"
assert_dir "$H/.config/workspacer-hub"         # vapid.json, push-subscriptions.json, jobs.json
assert_dir "$H/.config/git"
assert_dir "$H/.config/gh"
assert_dir "$H/.local/share/claudemon"         # state.db
assert_dir "$H/.workspacer/worktrees"          # must live or die with repos/
assert_dir "$H/.workspacer/handoffs"
assert_dir "$H/.ssh"
assert_dir "$D/repos"
assert_dir "$D/go/pkg/mod"
assert_dir "$D/go/cache"
assert_dir "$D/bundle"
assert_dir "$D/bun"
assert_dir "$D/npm"
assert_dir "$D/tailscale"                      # tailscaled.state — the node identity
assert_dir "$D/logs"
assert_dir "$D/state"

assert_mode "$H/.ssh" 700       # ssh refuses the keys otherwise
assert_mode "$D/tailscale" 700  # holds the node key

assert_file "$D/state/layout-version"
assert_file "$H/.bashrc"
assert_grep "$(cat "$H/.bashrc")" '.wks-env' "seeded .bashrc with the generated-env hook"
assert_file "$D/state/owner-uid"
assert_content "$D/state/owner-uid" "$(id -u):$(id -g)" "ownership marker written on first boot"

# ===========================================================================
section "SYMLINK RULE — zero symlinks are created, anywhere"
# ===========================================================================
# The rule: persist DIRECTORIES, never individual files. This design goes
# further and creates no symlinks at all, because $HOME itself is on the
# volume. Asserted mechanically so a future edit cannot quietly reintroduce one.
links="$(find "$D" -type l 2>/dev/null)"
if [ -z "$links" ]; then
  ok "no symlinks under the volume at all — nothing for a rename() to destroy"
else
  bad "symlinks found under the volume:"
  while IFS= read -r l; do printf '       %s\n' "$l"; done <<<"$links"

fi

# ===========================================================================
section "SYMLINK RULE — the mechanism it protects against"
# ===========================================================================
# An executable demonstration, so the rule is not folklore. Every atomic writer
# in this stack writes a sibling temp file and rename()s it over the target.
S="$TMP/symlink-demo"; mkdir -p "$S/volume" "$S/rootfs-home"

# (a) a FILE symlink — the pattern this design refuses.
echo "on the volume" >"$S/volume/claude.json"
ln -s "$S/volume/claude.json" "$S/rootfs-home/.claude.json"
echo "rewritten" >"$S/rootfs-home/.claude.json.tmp.4242"
mv "$S/rootfs-home/.claude.json.tmp.4242" "$S/rootfs-home/.claude.json"
if [ -L "$S/rootfs-home/.claude.json" ]; then
  bad "file symlink survived a tmp+rename (unexpected on this filesystem)"
else
  ok "a FILE symlink IS destroyed by tmp+rename — the write silently lands on the rootfs"
fi
assert_content "$S/volume/claude.json" "on the volume" "  ...and the volume copy is left stale, which is the silent data loss"

# (b) a DIRECTORY symlink — safe, because the rename happens inside it.
mkdir -p "$S/volume/claudedir"
echo "v1" >"$S/volume/claudedir/settings.json"
ln -s "$S/volume/claudedir" "$S/rootfs-home/.claude"
echo "v2" >"$S/rootfs-home/.claude/settings.json.tmp.4242"
mv "$S/rootfs-home/.claude/settings.json.tmp.4242" "$S/rootfs-home/.claude/settings.json"
if [ -L "$S/rootfs-home/.claude" ]; then
  ok "a DIRECTORY symlink SURVIVES the same write — the rename happens inside it"
else
  bad "directory symlink was destroyed (unexpected)"
fi
assert_content "$S/volume/claudedir/settings.json" "v2" "  ...and the new data landed on the volume"

# ===========================================================================
section "LATER BOOT — populated volume, nothing is clobbered"
# ===========================================================================
# Seed the volume the way a real one looks after a human has logged in and the
# daemons have run: credentials, a hand-edited config, a customised shell.
cat >"$H/.claude.json" <<'EOF'
{"hasCompletedOnboarding":true,"projects":{"/data/repos/workspacer":{"hasTrustDialogAccepted":true}}}
EOF
printf '[user]\n\tname = real human\n' >"$H/.gitconfig"
echo "projects: [/data/repos/workspacer]" >"$H/.config/workspacer/config.yaml"
echo "sqlite-bytes-would-go-here" >"$H/.local/share/claudemon/state.db"
echo "node-key-would-go-here" >"$D/tailscale/tailscaled.state"
echo "# my own alias" >>"$H/.bashrc"
before_bashrc="$(cat "$H/.bashrc")"

out="$(run_bootstrap "$D" WKS_SKIP_CHOWN=0)"; rc=$?
assert "$rc" "exits 0 on a populated volume"
assert_grep "$out" "populated volume" "recognises this as a later boot"
assert_not_grep "$out" "FIRST BOOT" "does NOT report a first boot"
assert_grep "$out" "skipped the deep chown" "skips the deep chown when the ownership marker already matches"

assert_grep "$(cat "$H/.claude.json")" 'hasTrustDialogAccepted' "HOME/.claude.json untouched (the folder-trust map that keeps a headless spawn off an unanswerable dialog)"
assert_grep "$(cat "$H/.gitconfig")" 'real human' "HOME/.gitconfig untouched"
assert_grep "$(cat "$H/.config/workspacer/config.yaml")" 'workspacer' "config.yaml untouched (brain reseeds it with DEFAULTS on ENOENT, silently)"
assert_content "$H/.local/share/claudemon/state.db" "sqlite-bytes-would-go-here" "claudemon state.db untouched"
assert_content "$D/tailscale/tailscaled.state" "node-key-would-go-here" "tailscaled.state untouched (the tailnet identity)"
assert_content "$H/.bashrc" "$before_bashrc" ".bashrc untouched — the .wks-env hook is appended once, not on every boot"
hooks="$(grep -cF '.wks-env' "$H/.bashrc")"
assert "$([ "$hooks" = 1 ] && echo 0 || echo 1)" ".wks-env hook appears exactly once after two boots (got $hooks)"

# Third boot for good measure: idempotence is not a two-run property.
out="$(run_bootstrap "$D" WKS_SKIP_CHOWN=0)"; rc=$?
assert "$rc" "exits 0 on a third boot"
assert_content "$H/.bashrc" "$before_bashrc" ".bashrc still untouched after a third boot"

# ===========================================================================
section "PARTIAL VOLUME — a directory went missing between boots"
# ===========================================================================
# Not first boot, but not intact either. This must be repaired AND said out
# loud: silence here reads as "everything was fine".
rm -rf "$H/.workspacer/handoffs" "$D/go/cache"
out="$(run_bootstrap "$D" WKS_SKIP_CHOWN=0)"; rc=$?
assert "$rc" "exits 0 after repairing a partial volume"
assert_dir "$H/.workspacer/handoffs"
assert_dir "$D/go/cache"
assert_grep "$out" "were MISSING and have been recreated" "reports the repair instead of staying silent"

# ===========================================================================
section "LAYOUT UPGRADE — volume built by an older image"
# ===========================================================================
echo "0" >"$D/state/layout-version"
out="$(run_bootstrap "$D" WKS_SKIP_CHOWN=0)"; rc=$?
assert "$rc" "exits 0 against an older layout version"
assert_grep "$out" "was built by layout v0" "notices the volume predates this image"
assert_content "$D/state/layout-version" "1" "records the new layout version"

# ===========================================================================
section "OWNERSHIP — first boot deep-chowns, later boots do not"
# ===========================================================================
# A Fly volume mounts root-owned and every state file this stack writes is 0600
# or 0700, so a uid mismatch is fatal rather than degraded. But `chown -R` over
# a 30GB volume is minutes of wake latency against a ~15s budget, so the deep
# pass is gated on a marker.
D2="$TMP/ownership"; mkdir -p "$D2"
out="$(run_bootstrap "$D2" WKS_SKIP_CHOWN=0)"
assert_grep "$out" "deep chown of" "first boot performs the deep chown"
out="$(run_bootstrap "$D2" WKS_SKIP_CHOWN=0)"
assert_not_grep "$out" "deep chown of" "second boot does not repeat it"

rm -f "$D2/state/owner-uid"
out="$(run_bootstrap "$D2" WKS_SKIP_CHOWN=0)"
assert_grep "$out" "deep chown of" "a missing marker (restored or rebuilt volume) re-runs the deep chown"

# ===========================================================================
printf '\n\033[1m%d passed, %d failed\033[0m\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
