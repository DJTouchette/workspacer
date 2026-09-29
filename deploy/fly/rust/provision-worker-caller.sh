#!/usr/bin/env bash
# Explicit, local, owner-operated migration. Never contacts Fly or another host.
# Usage: provision-worker-caller.sh TOKENS_JSON NODE_ID OUTPUT_FILE
set -euo pipefail
umask 077
[ "$#" = 3 ] || { printf '%s\n' 'usage: provision-worker-caller.sh TOKENS_JSON NODE_ID OUTPUT_FILE' >&2; exit 2; }
store=$1
node_id=$2
output=$3
[[ "$node_id" =~ ^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$ ]] || { printf '%s\n' 'invalid node id' >&2; exit 2; }
[ ! -e "$output" ] && [ ! -L "$output" ] || { printf '%s\n' 'refusing to replace an existing output credential' >&2; exit 2; }
[ -d "$(dirname -- "$output")" ] || { printf '%s\n' 'output parent directory must already exist' >&2; exit 2; }
owner_path=$store
[ -e "$owner_path" ] || owner_path=$(dirname -- "$store")
[ "$(stat -c %u -- "$owner_path")" = "$(id -u)" ] || { printf '%s\n' 'run this migration as the token store owner (wks in the Fly image), not a different uid' >&2; exit 2; }
cli=${WORKSPACER_RUST_BIN:-/usr/local/bin/workspacer-rust}
label="worker:${node_id}:mcp"
"$cli" --tokens-file "$store" --json token list | jq -e --arg label "$label" 'all(.[]; .label != $label)' >/dev/null || {
  printf '%s\n' 'worker service label already exists or token store could not be read; inspect/revoke it explicitly first' >&2; exit 2;
}
temporary=$(mktemp -- "${output}.new.XXXXXX")
prefix=
committed=0
cleanup() {
  if [ "$committed" != 1 ] && [ -n "$prefix" ]; then
    "$cli" --tokens-file "$store" token revoke "$prefix" >/dev/null || printf '%s\n' 'credential rollback failed; inspect the worker service label before retrying' >&2
  fi
  rm -f -- "$temporary"
}
trap cleanup EXIT
"$cli" --tokens-file "$store" token create --scope operator --label "$label" >"$temporary"
prefix=$(head -c 8 -- "$temporary")
[ "${#prefix}" = 8 ] || { printf '%s\n' 'credential creation failed' >&2; exit 1; }
"$cli" --tokens-file "$store" token facade-authority --label "$label" --enabled true >/dev/null
sync -f -- "$temporary"
# Same-directory hard link is an atomic create-if-absent; never replace a file
# that appeared after validation. A failed publication revokes this new token.
ln -- "$temporary" "$output"
committed=1
printf 'Created dedicated scoped facade credential at %s (label %s). Transfer it through your secret store; do not reuse the provider or owner token.\n' "$output" "$label"
