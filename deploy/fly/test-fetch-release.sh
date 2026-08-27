#!/usr/bin/env bash
#
# test-fetch-release.sh — exercise the ARTIFACT install path without a network,
# a GitHub release, or a docker build.
#
# fetch-release.sh is the only thing standing between "a box deploy is fast" and
# "a box deploy silently runs whatever was behind a mutable tag this morning".
# Every interesting case it handles is a case you would otherwise discover on a
# machine in ord, at the far end of a ten-minute image push, with nothing on the
# box able to tell you which commit it is running — which is the exact gap the
# build stamp exists to close.
#
# So the cases are asserted here instead. `curl` speaks file://, so the whole
# download-verify-install path runs for real against tarballs this script builds:
# no HTTP server, no fixtures checked in, no release required.
#
#   ./test-fetch-release.sh
#
# No root, no Docker, no network. Runs in about a second.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FETCH="$HERE/fetch-release.sh"
WRITE_STAMP="$HERE/write-build-stamp.sh"

pass=0
fail=0

section() { printf '\n\033[1m== %s\033[0m\n' "$1"; }
ok()   { pass=$((pass+1)); printf '  \033[32mPASS\033[0m %s\n' "$1"; }
bad()  { fail=$((fail+1)); printf '  \033[31mFAIL\033[0m %s\n' "$1"; }

assert_rc()       { if [ "$1" = "$2" ]; then ok "$3 (rc=$1)"; else bad "$3 (rc=$1, want $2)"; fi; }
assert_grep()     { if grep -qF "$2" <<<"$1"; then ok "$3"; else bad "$3 -- output was: $1"; fi; }
assert_not_grep() { if grep -qF "$2" <<<"$1"; then bad "$3"; else ok "$3"; fi; }
assert_file()     { if [ -f "$1" ]; then ok "$2"; else bad "$2 (missing $1)"; fi; }
assert_no_file()  { if [ -e "$1" ]; then bad "$2"; else ok "$2"; fi; }
assert_exec()     { if [ -x "$1" ]; then ok "$2"; else bad "$2 (not executable: $1)"; fi; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/wks-fetch-release-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

command -v curl >/dev/null 2>&1 || { echo "test-fetch-release.sh: curl is required"; exit 1; }

# ---------------------------------------------------------------------------
# A fake release: the same directory shape .github/workflows/release.yml builds,
# with the same stamp writer, so a change to the stamp FORMAT breaks this suite
# rather than breaking a box deploy.
# ---------------------------------------------------------------------------
# make_bundle <name> <tag> <commit> [omit...]
#   omit: file names to leave OUT (e.g. mcp, build-stamp) — the interesting cases
make_bundle() {
  local name="$1" tag="$2" commit="$3"; shift 3
  local omit=" $* "
  local root="$TMP/build/$name/workspacer-server"
  rm -rf "$TMP/build/$name"
  mkdir -p "$root/web/assets"
  local f
  for f in workspacer hub brain claudemon mcp; do
    case "$omit" in *" $f "*) continue ;; esac
    printf '#!/bin/sh\necho %s\n' "$f" >"$root/$f"
    # Deliberately NOT executable: the archive comes off a Windows-hostile
    # pipeline and a chmod is one of the things fetch-release.sh owes the image.
    chmod 0644 "$root/$f"
  done
  case "$omit" in *" web "*) rm -rf "$root/web" ;; *)
    echo '<!doctype html><title>app</title>' >"$root/web/index.html"
    echo 'console.log(1)' >"$root/web/assets/index.js" ;;
  esac
  case "$omit" in *" build-stamp "*) : ;; *)
    WKS_STAMP_COMPONENT=server WKS_STAMP_INSTALL=release \
    WKS_STAMP_VERSION=0.150.0-nightly.202608270800 \
    WKS_STAMP_TAG="$tag" WKS_STAMP_COMMIT="$commit" \
    WKS_STAMP_BUILT=2026-08-27T08:00:00Z WKS_STAMP_PLATFORM=linux-x64 \
    WKS_STAMP_RUN=12345 \
      bash "$WRITE_STAMP" "$root/build-stamp" >/dev/null ;;
  esac
  mkdir -p "$TMP/releases/$tag"
  tar -C "$TMP/build/$name" -czf "$TMP/releases/$tag/workspacer-server-linux-x64.tar.gz" workspacer-server
}

# Run fetch-release.sh against the fake release tree. $out / $rc are the result.
fetch() {
  local dest="$1"; shift
  rm -rf "$dest"
  out="$(env WKS_RELEASE_BASE_URL="file://$TMP/releases" \
             WKS_RELEASE_ASSET=workspacer-server-linux-x64.tar.gz \
             WKS_RELEASE_REPO=DJTouchette/workspacer \
             "$@" bash "$FETCH" "$dest" 2>&1)"
  rc=$?
}

REAL_SHA=1234567890abcdef1234567890abcdef12345678

# ===========================================================================
section "The happy path installs the same shape a source build produces"
# ===========================================================================
make_bundle good nightly "$REAL_SHA"
fetch "$TMP/out-good" WKS_RELEASE_TAG=nightly
assert_rc "$rc" 0 "a matching bundle installs"
assert_file "$TMP/out-good/workspacer" "workspacer is installed"
assert_file "$TMP/out-good/mcp"        "mcp is installed — the file the bundle used to be missing"
assert_file "$TMP/out-good/claudemon"  "claudemon is installed"
assert_file "$TMP/out-good/build-stamp" "the stamp travels with the files"
assert_file "$TMP/out-good/web/index.html" "the web app comes along"
assert_exec "$TMP/out-good/workspacer" "binaries are made executable (the archive's 0644 is not enough)"
assert_exec "$TMP/out-good/mcp"        "including mcp"
assert_grep "$out" "stamp verified: tag=nightly" "says which stamp it accepted"
assert_grep "$out" "sha256(archive)" "records the exact download in the build log"

# The stamp is installed VERBATIM. If this ever starts rewriting install=, the
# image loses the only record of whether its bits were compiled or downloaded.
assert_grep "$(cat "$TMP/out-good/build-stamp")" "install=release" \
  "the bundle's own provenance is preserved, not overwritten by the installer"

# ===========================================================================
section "THE DRIFT GUARD: a tag that does not match fails the build, loudly"
# ===========================================================================
# The case this is really about: `nightly` is deleted and recreated every night,
# so asking for it is not asking for a commit, and a cache or a mirror can hand
# back yesterday's. Nothing at runtime would ever show it.
make_bundle stale nightly "$REAL_SHA"
mkdir -p "$TMP/releases/v0.150.0"
cp "$TMP/releases/nightly/workspacer-server-linux-x64.tar.gz" "$TMP/releases/v0.150.0/"
fetch "$TMP/out-drift" WKS_RELEASE_TAG=v0.150.0
assert_rc "$rc" 1 "asking for v0.150.0 and getting a bundle stamped nightly fails"
assert_grep "$out" "RELEASE DRIFT" "names it as drift rather than a download error"
assert_grep "$out" "asked for tag 'v0.150.0'" "prints what was asked for"
assert_grep "$out" "the archive says it is 'nightly'" "and what arrived"
assert_no_file "$TMP/out-drift/workspacer" "and installs nothing"

# ===========================================================================
section "THE SHA GUARD: the right tag built from the wrong commit still fails"
# ===========================================================================
make_bundle othersha nightly deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
fetch "$TMP/out-sha" WKS_RELEASE_TAG=nightly WKS_RELEASE_SHA="$REAL_SHA"
assert_rc "$rc" 1 "a commit mismatch fails even though the tag matched"
assert_grep "$out" "COMMIT DRIFT" "names it"
assert_grep "$out" "gh release view" "and hands over the sha-to-release lookup"

make_bundle rightsha nightly "$REAL_SHA"
fetch "$TMP/out-sha-ok" WKS_RELEASE_TAG=nightly WKS_RELEASE_SHA="${REAL_SHA:0:12}"
assert_rc "$rc" 0 "a SHORT sha is accepted — nobody types forty characters"
assert_grep "$out" "commit verified" "and says the commit was checked"

fetch "$TMP/out-sha-short" WKS_RELEASE_TAG=nightly WKS_RELEASE_SHA=123
assert_rc "$rc" 1 "a sha too short to mean anything is refused rather than 'matched'"
assert_grep "$out" "fewer than 7 characters" "and says why"

# ===========================================================================
section "A bundle with no stamp is refused, not installed hopefully"
# ===========================================================================
# This is every release published before the stamping step landed. Installing it
# would produce exactly the unidentifiable box the stamp exists to prevent, so
# the refusal points at the one mode that CAN build an old commit.
make_bundle nostamp nightly "$REAL_SHA" build-stamp
fetch "$TMP/out-nostamp" WKS_RELEASE_TAG=nightly
assert_rc "$rc" 1 "an unstamped bundle fails the build"
assert_grep "$out" "carries no build-stamp" "says the stamp is what is missing"
assert_grep "$out" "WKS_INSTALL=source" "and names the way out"

# ===========================================================================
section "COMPLETENESS: a bundle missing a binary fails here, not at 3am on boot"
# ===========================================================================
# `mcp` is the concrete one: the node entrypoint starts the MCP facade before the
# brain and `die`s if it does not answer within ten seconds. An image built from
# a bundle without it builds green and cannot boot.
make_bundle nomcp nightly "$REAL_SHA" mcp
fetch "$TMP/out-nomcp" WKS_RELEASE_TAG=nightly
assert_rc "$rc" 1 "a bundle without mcp is refused"
assert_grep "$out" "missing 1 file(s) this image needs: mcp" "names the missing file"
assert_grep "$out" "Bundle standalone server" "and points at the release step that owes it"

# The hub asks for a different set, and gets its own answer.
make_bundle noweb nightly "$REAL_SHA" web
fetch "$TMP/out-noweb" WKS_RELEASE_TAG=nightly \
  WKS_RELEASE_REQUIRE="hub web/index.html build-stamp" WKS_RELEASE_CHMOD=hub
assert_rc "$rc" 1 "the hub's own require-list catches a bundle with no web app"
assert_grep "$out" "web/index.html" "naming the path it wanted"

make_bundle hubok nightly "$REAL_SHA"
fetch "$TMP/out-hubok" WKS_RELEASE_TAG=nightly \
  WKS_RELEASE_REQUIRE="hub web/index.html build-stamp" WKS_RELEASE_CHMOD=hub
assert_rc "$rc" 0 "and passes on a complete one"
assert_exec "$TMP/out-hubok/hub" "chmodding only what it was asked to chmod"

# ===========================================================================
section "Operator mistakes get a sentence, not a stack trace"
# ===========================================================================
fetch "$TMP/out-notag" WKS_RELEASE_TAG=
assert_rc "$rc" 1 "an empty tag refuses immediately"
assert_grep "$out" "WKS_RELEASE_TAG is empty" "and says which arg is missing"

fetch "$TMP/out-404" WKS_RELEASE_TAG=v9.9.9-does-not-exist
assert_rc "$rc" 1 "a tag with no asset behind it fails"
assert_grep "$out" "could not download" "as a download failure"
assert_grep "$out" "gh release view" "with the command that checks the tag"

# ===========================================================================
section "The stamp format is the contract, and it is one writer"
# ===========================================================================
# Every reader — this script, verify-image.sh, both entrypoints — greps these
# keys. A key that disappears breaks all four at once, silently, so the key set
# is asserted rather than assumed.
WKS_STAMP_COMPONENT=hub WKS_STAMP_INSTALL=source WKS_STAMP_COMMIT=abc123 \
  bash "$WRITE_STAMP" "$TMP/stamp-min" >/dev/null
stamp="$(cat "$TMP/stamp-min")"
for key in component install version tag commit built platform run; do
  assert_grep "$stamp" "$key=" "the stamp always carries $key="
done
assert_grep "$stamp" "version=unknown" "an unknown value is written as 'unknown', never omitted"
assert_grep "$stamp" "component=hub" "and the caller's values land"
assert_not_grep "$stamp" "#" "no comments — it is read by grep|cut in a stage with no jq"
lines="$(wc -l <"$TMP/stamp-min")"
if [ "$lines" = 8 ]; then ok "exactly 8 lines, one per key"; else bad "8 lines expected, got $lines"; fi
# The entrypoints print it as ONE log line via tr '\n' ' '.
assert_grep "$(tr '\n' ' ' <"$TMP/stamp-min")" "component=hub install=source" \
  "and folds onto a single boot-log line"

# ===========================================================================
printf '\n\033[1m%d passed, %d failed\033[0m\n' "$pass" "$fail"
[ "$fail" -eq 0 ] || exit 1
