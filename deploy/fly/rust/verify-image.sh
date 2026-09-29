#!/usr/bin/env bash
set -euo pipefail
role="${1:?node, hub or combined required}"
case "$role" in node|hub|combined) ;; *) exit 2 ;; esac
for binary in workspacer-rust tailscale tailscaled tini setpriv jq curl node npm; do
  command -v "$binary" >/dev/null || { printf 'missing runtime binary: %s\n' "$binary" >&2; exit 1; }
done
if [ "$role" != hub ]; then command -v claudemon >/dev/null; command -v claude >/dev/null; fi
for obsolete in hub brain mcp; do
  if command -v "$obsolete" >/dev/null 2>&1; then printf 'unexpected legacy runtime: %s\n' "$obsolete" >&2; exit 1; fi
done
[ "$(readlink -f /usr/local/bin/workspacer)" = /usr/local/bin/workspacer-rust ]
[ "$(id -u wks)" = 10001 ] && [ "$(id -g wks)" = 10001 ]
[ "$(id -u)" = 0 ] # entrypoint must prepare the volume/network before dropping uid
[ -s /usr/local/share/workspacer/build-stamp ]
if [ -d /data ] && [ -n "$(find /data -mindepth 1 -print -quit)" ]; then
  printf '%s\n' 'image must not bake state into the mounted volume path' >&2; exit 1
fi
for path in /root/.claude /root/.codex /root/.config/workspacer /root/.local/share/claudemon; do
  [ ! -e "$path" ] || { printf 'unexpected built-in account state: %s\n' "$path" >&2; exit 1; }
done
workspacer-rust serve --help >/dev/null

[ ! -e /usr/local/bin/desktop-host.cjs ]
