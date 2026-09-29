#!/usr/bin/env bash
# Exercise the actual release fetcher against a local Rust-only archive.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/tree/workspacer-server/web" "$fixture/releases/nightly"
printf '#!/bin/sh\nexit 0\n' >"$fixture/tree/workspacer-server/workspacer-rust"
printf '#!/bin/sh\nexit 0\n' >"$fixture/tree/workspacer-server/claudemon"
printf '<html>fixture</html>\n' >"$fixture/tree/workspacer-server/web/index.html"
WKS_STAMP_COMPONENT=rust-server WKS_STAMP_INSTALL=release WKS_STAMP_TAG=nightly \
  WKS_STAMP_COMMIT=1234567890abcdef1234567890abcdef12345678 \
  bash "$root/write-build-stamp.sh" "$fixture/tree/workspacer-server/build-stamp" >/dev/null
pack() { tar -C "$fixture/tree" -czf "$fixture/releases/nightly/workspacer-server-linux-x64.tar.gz" workspacer-server; }
fetch() {
  WKS_RELEASE_BASE_URL="file://$fixture/releases" WKS_RELEASE_TAG=nightly \
    WKS_RELEASE_SHA=1234567890abcdef1234567890abcdef12345678 \
    WKS_RELEASE_REQUIRE='workspacer-rust claudemon web/index.html build-stamp' \
    WKS_RELEASE_CHMOD='workspacer-rust claudemon' \
    bash "$root/fetch-release.sh" "$1"
}
pack
fetch "$fixture/installed" >"$fixture/log" 2>&1
[ -x "$fixture/installed/workspacer-rust" ] && [ -x "$fixture/installed/claudemon" ]
[ ! -e "$fixture/installed/brain" ] && [ ! -e "$fixture/installed/mcp" ]
rm "$fixture/tree/workspacer-server/workspacer-rust"
printf '#!/bin/sh\nexit 0\n' >"$fixture/tree/workspacer-server/workspacer"
pack
if fetch "$fixture/refused" >"$fixture/refused-log" 2>&1; then echo 'legacy-only artifact unexpectedly accepted' >&2; exit 1; fi
[ ! -e "$fixture/refused/workspacer" ]
printf '%s\n' 'PASS: Rust-only release installs and legacy-only bundle refuses without fallback'
