#!/usr/bin/env bash
#
# test-bootstrap.sh — exercise the hub's bootstrap.sh against a temp directory
# standing in for /data.
#
# The node's version of this file exists to prove a volume layout is idempotent.
# This one has a harder job, because the hub's interesting failures are not
# "it crashed" — they are "it came up looking healthy against the wrong state".
#
# A hub that restarts without its state does not fail. It re-mints the pairing
# credential and refuses every client it has ever paired with; it regenerates the
# Web Push keypair and kills every subscription while each phone still reports
# itself subscribed; it finds no node registry and simply never registers
# nodes.wake. Every one of those presents as a working hub.
#
# So what is asserted here is the DIFFERENCE between an empty volume and a
# damaged one — the same code path, two situations, opposite correct answers —
# on a laptop, rather than discovered at 3am on a machine holding a token that
# spends money.
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
assert_no_file()  { if [ -e "$1" ]; then bad "file should NOT exist: ${1#"$TMP"}"; else ok "absent as expected: ${1#"$TMP"}"; fi; }
assert_mode()     { local m; m="$(stat -c '%a' "$1")"; if [ "$m" = "$2" ]; then ok "mode $2 on ${1#"$TMP"}"; else bad "mode on ${1#"$TMP"} is $m, want $2"; fi; }
assert_content()  { local got; got="$(cat "$1" 2>/dev/null)"; if [ "$got" = "$2" ]; then ok "$3"; else bad "$3 (got: '$got')"; fi; }
assert_grep()     { if grep -qF "$2" <<<"$1"; then ok "$3"; else bad "$3"; fi; }
assert_not_grep() { if grep -qF "$2" <<<"$1"; then bad "$3"; else ok "$3"; fi; }
assert_rc()       { if [ "$1" = "$2" ]; then ok "$3 (rc=$1)"; else bad "$3 (rc=$1, want $2)"; fi; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/wks-hub-bootstrap-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# The volume is not a real mountpoint here, so every run passes
# WKS_REQUIRE_MOUNT=0 except the case that specifically tests the refusal.
# HUB_TOKEN is explicitly cleared unless a case sets it: it is very likely to be
# present in a developer's real environment, and it changes the code path.
run_bootstrap() {
  local data="$1"; shift
  env -u HUB_TOKEN -u WORKSPACER_ALLOW_NEW_TOKEN -u WKS_ALLOW_STATE_LOSS -u WKS_ALLOW_TOKEN_CHANGE \
      WKS_DATA="$data" WKS_HOME="$data/home" \
      WKS_UID="$(id -u)" WKS_GID="$(id -g)" \
      WKS_REQUIRE_MOUNT=0 "$@" bash "$BOOTSTRAP" 2>&1
}

# ===========================================================================
section "refuses to run without the volume"
# ===========================================================================
# On this machine the rootfs case is worse than on the node: a hub there mints a
# new pairing credential, regenerates the push keypair, and finds no node
# registry — three silent identity changes and no way to wake anything.

out="$(env -u HUB_TOKEN WKS_DATA="$TMP/does-not-exist" WKS_REQUIRE_MOUNT=0 bash "$BOOTSTRAP" 2>&1)"; rc=$?
assert "$([ $rc -ne 0 ] && echo 0 || echo 1)" "nonzero exit when \$WKS_DATA does not exist (rc=$rc)"
assert_grep "$out" "does not exist" "says the volume is not mounted"

mkdir -p "$TMP/not-a-mount"
out="$(env -u HUB_TOKEN WKS_DATA="$TMP/not-a-mount" WKS_REQUIRE_MOUNT=1 bash "$BOOTSTRAP" 2>&1)"; rc=$?
assert "$([ $rc -ne 0 ] && echo 0 || echo 1)" "nonzero exit when \$WKS_DATA exists but is not a mountpoint (rc=$rc)"
assert_grep "$out" "not a mountpoint" "distinguishes 'not mounted' from 'missing'"

# ===========================================================================
section "FIRST BOOT — empty volume"
# ===========================================================================
D="$TMP/empty"; H="$D/home"; C="$H/.config"
mkdir -p "$D"
out="$(run_bootstrap "$D")"; rc=$?

assert_rc "$rc" 0 "exits 0 on an empty volume"
assert_grep "$out" "FIRST BOOT" "announces the first boot"

# Every directory, verified against the source that reads it.
assert_dir "$C/workspacer"                     # authtoken.ConfigDir(): remote-token, tokens.json, peers.json, nodes.json
assert_dir "$C/workspacer/plugins"             # the hub is what loads plugins
assert_dir "$C/workspacer/library"
assert_dir "$C/workspacer/layouts"
assert_dir "$C/workspacer/sessions"
assert_dir "$C/workspacer-hub"                 # defaultPushDir/defaultJobsFile/defaultLayoutFile
assert_dir "$D/tailscale"                      # tailscaled.state — the hub's tailnet identity
assert_dir "$D/logs"
assert_dir "$D/state"
assert_dir "$D/state/seen"                     # the per-file "this has existed" markers

assert_mode "$D/tailscale" 700                 # holds the node key
assert_mode "$C/workspacer-hub" 700            # holds the VAPID private key

# What is deliberately NOT here. The hub runs no agents, so it holds no agent
# credentials — the always-on machine is the one with the least on it.
assert_no_file "$H/.claude"
assert_no_file "$H/.claude.json"
assert_no_file "$H/.codex"
assert_no_file "$H/.ssh"
assert_no_file "$D/repos"

assert_file "$D/state/layout-version"
assert_file "$H/.bashrc"
assert_grep "$(cat "$H/.bashrc")" '.wks-env' "seeded .bashrc with the generated-env hook"
assert_file "$D/state/owner-uid"

# ===========================================================================
section "FIRST RUN — the pairing credential is minted, loudly"
# ===========================================================================
assert_file "$C/workspacer/remote-token"
assert_mode "$C/workspacer/remote-token" 600   # it is a bearer secret, like an SSH key
assert_grep "$out" "FIRST RUN: minted a new pairing credential" "says a credential was minted"
assert_grep "$out" "HUB_TOKEN=" "prints it, because the operator has to pair with it"

tok="$(cat "$C/workspacer/remote-token")"
# 24 random bytes as base64url with no padding = 32 chars. Same shape as
# loadOrCreateToken's crypto/rand + base64.RawURLEncoding, and the desktop's.
assert "$([ "${#tok}" = 32 ] && echo 0 || echo 1)" "minted token is 32 chars — 24 bytes base64url, the shape loadOrCreateToken produces (got ${#tok})"
if [[ "$tok" =~ ^[A-Za-z0-9_-]+$ ]]; then ok "minted token is base64url (no + / =)"; else bad "minted token is not base64url: $tok"; fi
assert_file "$D/state/seen/config_workspacer_remote-token"

# ===========================================================================
section "FIRST RUN — an absent nodes.json is NOT an error"
# ===========================================================================
# It becomes one only once this volume has held one. An operator running an
# always-on hub with no remote nodes must not be refused on every boot.
assert_grep "$out" "nodes.json: absent, and nothing on this volume says it ever existed" \
  "treats a never-seen nodes.json as a first run, not a loss"
assert_not_grep "$out" "STATE LOSS" "no state loss is reported on an empty volume"

# ===========================================================================
section "LATER BOOT — populated volume, nothing is clobbered"
# ===========================================================================
# Seed the volume the way a real one looks after provisioning.
cat >"$C/workspacer/nodes.json" <<'EOF'
[{"id":"fly-node","label":"Fly node (ord)","fly":{"app":"workspacer-node","machineId":"17811944b12345"}}]
EOF
chmod 600 "$C/workspacer/nodes.json"
echo '{"publicKey":"pub","privateKey":"priv"}' >"$C/workspacer-hub/vapid.json"
echo '{"endpoint":"https://push.example/x"}'   >"$C/workspacer-hub/push-subscriptions.json"
echo '[{"name":"work","url":"ws://x/bus","token":"t"}]' >"$C/workspacer/peers.json"
echo '[]' >"$C/workspacer-hub/jobs.json"
echo 'projects: []' >"$C/workspacer/config.yaml"
echo 'node-key-would-go-here' >"$D/tailscale/tailscaled.state"
echo "# my own alias" >>"$H/.bashrc"
before_bashrc="$(cat "$H/.bashrc")"

out="$(run_bootstrap "$D")"; rc=$?
assert_rc "$rc" 0 "exits 0 on a populated volume"
assert_grep "$out" "populated volume" "recognises this as a later boot"
assert_not_grep "$out" "FIRST BOOT" "does NOT report a first boot"
assert_grep "$out" "state guard: no losses detected" "reports a clean state guard"

assert_content "$C/workspacer/remote-token" "$tok" "the pairing credential is UNTOUCHED — the whole point"
assert_grep "$(cat "$C/workspacer/nodes.json")" '17811944b12345' "nodes.json untouched"
assert_grep "$(cat "$C/workspacer-hub/vapid.json")" 'priv' "vapid.json untouched (a regenerated keypair kills every push subscription)"
assert_content "$D/tailscale/tailscaled.state" "node-key-would-go-here" "tailscaled.state untouched (the tailnet identity)"
assert_content "$H/.bashrc" "$before_bashrc" ".bashrc untouched — the .wks-env hook is appended once, not on every boot"
hooks="$(grep -cF '.wks-env' "$H/.bashrc")"
assert "$([ "$hooks" = 1 ] && echo 0 || echo 1)" ".wks-env hook appears exactly once after two boots (got $hooks)"
assert_not_grep "$out" "HUB_TOKEN=" "does NOT reprint the credential on an ordinary boot"

# Every guarded file now has its marker. That is the evidence the next section uses.
for m in config_workspacer_nodes.json config_workspacer-hub_vapid.json \
         config_workspacer_peers.json config_workspacer-hub_jobs.json; do
  assert_file "$D/state/seen/$m"
done
# tailscaled.state lives outside $HOME. Its marker name must still not carry the
# mount path: a slug that did would orphan every marker if $WKS_DATA ever moved,
# and an orphaned marker fails OPEN: a real loss would read as a first run.
assert_file "$D/state/seen/tailscale_tailscaled.state"

# The same property, stated as the thing that would break: a marker set survives
# being moved to a different mount point. That is what a restored volume is.
DMOVE="$TMP/moved"
cp -a "$D" "$DMOVE"
rm -f "$DMOVE/home/.config/workspacer-hub/vapid.json"
out="$(run_bootstrap "$DMOVE")"
assert_grep "$out" "vapid.json is GONE" "markers still bite after the volume is mounted somewhere else"

# Third boot: idempotence is not a two-run property.
out="$(run_bootstrap "$D")"; rc=$?
assert_rc "$rc" 0 "exits 0 on a third boot"
assert_content "$C/workspacer/remote-token" "$tok" "credential still untouched after a third boot"

# ===========================================================================
section "A file that NEVER existed is not a loss, however full its neighbours"
# ===========================================================================
# The failure mode this pins is a guard that cries wolf. internal/statelost
# infers loss from "the directory still holds the rest of the state", which is
# sound only when running the hub GUARANTEES the file exists. That holds for
# remote-token and for nothing else: tokens.json exists only once somebody mints
# a scoped token, layout.json only once a client saves a layout, jobs-history
# only once a job runs. Judging those by their neighbours reports loss on every
# boot of a perfectly healthy hub — and a guard that is always wrong gets
# switched off, taking the two real refusals with it.
assert_no_file "$C/workspacer/tokens.json"
assert_no_file "$C/workspacer-hub/layout.json"
assert_no_file "$C/workspacer-hub/jobs-history.json"
assert_not_grep "$out" "tokens.json is GONE" "a never-minted tokens.json is not reported lost, though remote-token and nodes.json sit beside it"
assert_not_grep "$out" "layout.json is GONE" "a never-saved layout.json is not reported lost"
assert_not_grep "$out" "jobs-history.json is GONE" "a never-written jobs-history.json is not reported lost"

# ===========================================================================
section "STATE LOSS — the pairing credential vanishes: REFUSE"
# ===========================================================================
# This is the case the whole file exists for. Re-minting is not a recovery, it is
# a new identity: it revokes every existing pairing at once and the hub then
# prints a healthy banner. A phone that stopped working and a hub that was never
# provisioned look identical from outside.
D2="$TMP/lost-token"; H2="$D2/home"; C2="$H2/.config"
cp -a "$D" "$D2"
rm -f "$C2/workspacer/remote-token"

out="$(run_bootstrap "$D2")"; rc=$?
assert_rc "$rc" 2 "REFUSES to start (exit 2) when remote-token vanished from a volume that held it"
assert_grep "$out" "STATE LOSS" "names it as state loss, not a first run"
assert_grep "$out" "remote-token" "names the file"
assert_grep "$out" "REVOKES every existing pairing" "says what starting anyway would cost"
assert_grep "$out" "WORKSPACER_ALLOW_NEW_TOKEN=1" "names the one-word way past"
assert_no_file "$C2/workspacer/remote-token"

# The marker is the evidence, and it is stronger than the directory inference.
# Prove the marker alone is sufficient by emptying the directory around the file.
D3="$TMP/lost-token-bare"; C3="$D3/home/.config"
cp -a "$D" "$D3"
# -rf, not -f. `rm -f` does not remove directories, so plugins/, library/,
# layouts/, sessions/ and logs/ survived it, and this assertion is precisely
# the one claiming the marker works WITHOUT the directory inference. With five
# subdirectories still standing, the neighbours were still there and the test
# passed for a weaker reason than it stated (and printed five rm errors doing
# it). Emptying it for real is what makes the marker the only evidence left.
rm -rf "${C3:?}/workspacer/"*
out="$(run_bootstrap "$D3")"; rc=$?
assert_rc "$rc" 2 "still refuses when the whole config dir was emptied — the marker outlives the directory"
assert_grep "$out" "this volume has held it before" "cites the marker, not the neighbours"

# ===========================================================================
section "STATE LOSS — accepted deliberately, once"
# ===========================================================================
out="$(run_bootstrap "$D2" WORKSPACER_ALLOW_NEW_TOKEN=1)"; rc=$?
assert_rc "$rc" 0 "WORKSPACER_ALLOW_NEW_TOKEN=1 lets it start"
assert_file "$C2/workspacer/remote-token"
newtok="$(cat "$C2/workspacer/remote-token")"
assert "$([ "$newtok" != "$tok" ] && echo 0 || echo 1)" "a genuinely NEW credential was minted"
assert_grep "$out" "NEW PAIRING CREDENTIAL minted" "says a new identity was taken, not that state was restored"
assert_not_grep "$out" "FIRST RUN:" "does not mislabel a recovery as a first run"

# ===========================================================================
section "STATE LOSS — nodes.json vanishes: REFUSE"
# ===========================================================================
# The quietest failure available on this machine. nodes.LoadFile returns (nil,nil)
# for a missing file and startNodes returns before registering anything, so the
# hub comes up healthy with no nodes.list, no nodes.wake and no reconcile loop —
# and says so nowhere. That is exactly "cannot do the one job it is always on for".
D4="$TMP/lost-nodes"; C4="$D4/home/.config"
cp -a "$D" "$D4"
rm -f "$C4/workspacer/nodes.json"
out="$(run_bootstrap "$D4")"; rc=$?
assert_rc "$rc" 2 "REFUSES to start when nodes.json vanished from a volume that held it"
assert_grep "$out" "nodes.json" "names the file"
assert_grep "$out" "no error, no" "explains that the hub itself would not complain"

out="$(run_bootstrap "$D4" WKS_ALLOW_STATE_LOSS=1)"; rc=$?
assert_rc "$rc" 0 "WKS_ALLOW_STATE_LOSS=1 downgrades every refusal to a warning"
assert_grep "$out" "STATE LOSS" "still says loudly what was lost"

# ===========================================================================
section "STATE LOSS — vapid.json vanishes: WARN, do not refuse"
# ===========================================================================
# The asymmetry is deliberate and is push.New's own argument: stopping the bus,
# the sessions and federation because a NOTIFICATIONS keypair went missing is a
# far larger outage than the one being reported.
D5="$TMP/lost-vapid"; C5="$D5/home/.config"
cp -a "$D" "$D5"
rm -f "$C5/workspacer-hub/vapid.json"
out="$(run_bootstrap "$D5")"; rc=$?
assert_rc "$rc" 0 "starts anyway — a lost push keypair must not take the bus down with it"
assert_grep "$out" "vapid.json" "names the file"
assert_grep "$out" "still reports itself subscribed" "names the silent half of the damage"
assert_grep "$out" "restore it and restart NOW" "says the recovery window closes when devices re-subscribe"

# ===========================================================================
section "\$HUB_TOKEN — the recommended shape, and the conflict it can cause"
# ===========================================================================
# Supplying the credential as a Fly secret is the better design: it then outlives
# the volume and rotates with `fly secrets set`. But a secret that DISAGREES with
# the volume is the same identity change by another route.
D6="$TMP/secret-token"; C6="$D6/home/.config"
mkdir -p "$D6"
out="$(run_bootstrap "$D6" HUB_TOKEN=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)"; rc=$?
assert_rc "$rc" 0 "a first boot with \$HUB_TOKEN set exits 0"
assert_content "$C6/workspacer/remote-token" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
  "mirrors \$HUB_TOKEN onto the volume so the CLI on this machine agrees with what the hub serves"
assert_mode "$C6/workspacer/remote-token" 600
assert_not_grep "$out" "FIRST RUN: minted" "does not mint when the operator supplied one"

out="$(run_bootstrap "$D6" HUB_TOKEN=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)"; rc=$?
assert_rc "$rc" 0 "a matching \$HUB_TOKEN on a later boot is a no-op"

out="$(run_bootstrap "$D6" HUB_TOKEN=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)"; rc=$?
assert_rc "$rc" 2 "REFUSES when \$HUB_TOKEN disagrees with the credential on the volume"
assert_grep "$out" "IDENTITY CONFLICT" "names it as a conflict rather than a first run"
assert_grep "$out" "WKS_ALLOW_TOKEN_CHANGE=1" "names the way past"
assert_content "$C6/workspacer/remote-token" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
  "  ...and does NOT overwrite the volume's copy while refusing"

out="$(run_bootstrap "$D6" HUB_TOKEN=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb WKS_ALLOW_TOKEN_CHANGE=1)"; rc=$?
assert_rc "$rc" 0 "WKS_ALLOW_TOKEN_CHANGE=1 adopts the new secret"
assert_content "$C6/workspacer/remote-token" "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" "  ...and rewrites the volume's copy"

# ===========================================================================
section "A TRUNCATED credential is loss, not presence"
# ===========================================================================
# internal/statelost's own tests treat a zero-byte file beside real state as
# loss, and loadOrCreateToken discards a whitespace-only remote-token. A guard
# that only checked existence would sail straight past this.
D7="$TMP/truncated"; C7="$D7/home/.config"
cp -a "$D" "$D7"
: >"$C7/workspacer/remote-token"
out="$(run_bootstrap "$D7")"; rc=$?
assert_rc "$rc" 2 "an EMPTY remote-token is treated as loss, not as a present file"

# ===========================================================================
section "nodes.json permissions are repaired, because it may hold a cloud token"
# ===========================================================================
# The hub warns about an over-readable registry only after it has already been
# readable. Repairing the mode before the hub starts is strictly earlier.
D8="$TMP/perms"; C8="$D8/home/.config"
cp -a "$D" "$D8"
chmod 644 "$C8/workspacer/nodes.json"
out="$(run_bootstrap "$D8")"; rc=$?
assert_rc "$rc" 0 "exits 0"
assert_mode "$C8/workspacer/nodes.json" 600
assert_grep "$out" "tightened to 600" "reports the repair rather than doing it silently"

# ===========================================================================
section "SYMLINK RULE — zero symlinks are created, anywhere"
# ===========================================================================
# Persist DIRECTORIES, never individual files. This design goes further and
# creates no symlinks at all, because $HOME itself is on the volume. Asserted
# mechanically so a future edit cannot quietly reintroduce one.
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

echo "on the volume" >"$S/volume/vapid.json"
ln -s "$S/volume/vapid.json" "$S/rootfs-home/vapid.json"
echo "rewritten" >"$S/rootfs-home/vapid.json.tmp.4242"
mv "$S/rootfs-home/vapid.json.tmp.4242" "$S/rootfs-home/vapid.json"
if [ -L "$S/rootfs-home/vapid.json" ]; then
  bad "file symlink survived a tmp+rename (unexpected on this filesystem)"
else
  ok "a FILE symlink IS destroyed by tmp+rename — the write silently lands on the rootfs"
fi
assert_content "$S/volume/vapid.json" "on the volume" "  ...and the volume copy is left stale, which is the silent data loss"

mkdir -p "$S/volume/hubdir"
echo "v1" >"$S/volume/hubdir/jobs.json"
ln -s "$S/volume/hubdir" "$S/rootfs-home/workspacer-hub"
echo "v2" >"$S/rootfs-home/workspacer-hub/jobs.json.tmp.4242"
mv "$S/rootfs-home/workspacer-hub/jobs.json.tmp.4242" "$S/rootfs-home/workspacer-hub/jobs.json"
if [ -L "$S/rootfs-home/workspacer-hub" ]; then
  ok "a DIRECTORY symlink SURVIVES the same write — the rename happens inside it"
else
  bad "directory symlink was destroyed (unexpected)"
fi
assert_content "$S/volume/hubdir/jobs.json" "v2" "  ...and the new data landed on the volume"

# ===========================================================================
section "PARTIAL VOLUME — a directory went missing between boots"
# ===========================================================================
D9="$TMP/partial"
cp -a "$D" "$D9"
rm -rf "$D9/home/.config/workspacer/layouts" "$D9/home/.config/workspacer/library"
out="$(run_bootstrap "$D9")"; rc=$?
assert_rc "$rc" 0 "exits 0 after repairing a partial volume"
assert_dir "$D9/home/.config/workspacer/layouts"
assert_grep "$out" "were MISSING and have been recreated" "reports the repair instead of staying silent"

# ===========================================================================
section "LAYOUT UPGRADE — volume built by an older image"
# ===========================================================================
DA="$TMP/upgrade"
cp -a "$D" "$DA"
echo "0" >"$DA/state/layout-version"
out="$(run_bootstrap "$DA")"; rc=$?
assert_rc "$rc" 0 "exits 0 against an older layout version"
assert_grep "$out" "was built by layout v0" "notices the volume predates this image"
assert_content "$DA/state/layout-version" "1" "records the new layout version"

# ===========================================================================
section "OWNERSHIP — first boot chowns and writes the marker"
# ===========================================================================
# A Fly volume mounts root-owned, and every state file here is 0600 or 0700, so a
# uid mismatch after a fresh volume or a snapshot restore is fatal rather than
# degraded: push.New returns an error the hub treats as fatal at startup.
DB="$TMP/ownership"; mkdir -p "$DB"
out="$(run_bootstrap "$DB")"
assert_grep "$out" "chowning" "first boot performs the chown"
assert_content "$DB/state/owner-uid" "$(id -u):$(id -g)" "ownership marker written on first boot"
out="$(run_bootstrap "$DB")"
assert_grep "$out" "ownership marker matches" "a later boot takes the marker-matched path"

rm -f "$DB/state/owner-uid"
out="$(run_bootstrap "$DB")"
assert_grep "$out" "chowning" "a missing marker (restored or rebuilt volume) re-runs the chown"

# ===========================================================================
printf '\n\033[1m%d passed, %d failed\033[0m\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
