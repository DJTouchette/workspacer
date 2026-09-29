#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export WORKSPACER_RUST_BIN=${1:?path to built workspacer-rust required}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
store="$fixture/tokens.json"
output="$fixture/worker-caller"
"$root/provision-worker-caller.sh" "$store" worker-1 "$output" >"$fixture/log"
[ "$(stat -c %a "$output")" = 600 ]
jq -e --rawfile credential "$output" 'length == 1 and .[0].scope == "operator" and .[0].facadeAuthority == true and .[0].token == ($credential|rtrimstr("\n"))' "$store" >/dev/null
if grep -qF -f "$output" "$fixture/log"; then printf '%s\n' 'secret leaked to migration output' >&2; exit 1; fi
before=$(sha256sum "$output")
if "$root/provision-worker-caller.sh" "$store" worker-1 "$output" >/dev/null 2>&1; then exit 1; fi
[ "$before" = "$(sha256sum "$output")" ]
if "$root/provision-worker-caller.sh" "$store" worker-1 "$fixture/duplicate-label" >/dev/null 2>&1; then exit 1; fi
[ ! -e "$fixture/duplicate-label" ]
jq -e 'length == 1' "$store" >/dev/null
printf '%s\n' 'PASS: explicit worker facade grant, private output, no rotation or duplicate-label mutation'
