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
[ "$(id -u wks)" = "${WKS_UID:-10001}" ] && [ "$(id -g wks)" = "${WKS_GID:-10001}" ]
[ "$(getent passwd wks | cut -d: -f6)" = /data/home ]
case ":$PATH:" in *:/usr/local/go/bin:*) ;; *) echo 'reserved downstream Go toolchain path missing' >&2; exit 1 ;; esac
for variable in HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_CACHE_HOME \
  GOPATH GOMODCACHE GOCACHE BUNDLE_PATH BUN_INSTALL_CACHE_DIR npm_config_cache; do
  value=$(printenv "$variable" 2>/dev/null || true)
  case "$value" in /data|/data/*) echo "stateful environment variable baked into image: $variable" >&2; exit 1 ;; esac
done
for home_root in /root /home/wks /data/home; do
  for directory in .bun .cargo .rustup .deno .nvm .rbenv .rvm .pyenv .asdf .mise \
    .gem .bundle .local/bin .local/share/gem .local/share/mise .npm-global .yarn \
    .pnpm-store .volta .sdkman .opam .stack .ghcup .dotnet node_modules; do
    [ ! -e "$home_root/$directory" ] || { echo "toolchain installed into shadowed home: $home_root/$directory" >&2; exit 1; }
  done
done
[ ! -e /home/wks ] && [ ! -e /data/home ]
if [ "$role" = hub ]; then entrypoints=/usr/local/lib/wks-hub; else entrypoints=/usr/local/lib/wks; fi
[ -x "$entrypoints/entrypoint.sh" ] && [ -x "$entrypoints/bootstrap.sh" ]
if [ "$role" = combined ]; then [ -x /usr/local/lib/wks-combined/entrypoint.sh ]; fi
[ "$(id -u)" = 0 ] # entrypoint must prepare the volume/network before dropping uid
[ -s /usr/local/share/workspacer/build-stamp ]
for key in component install version tag commit built platform run; do
  grep -Eq "^${key}=.+$" /usr/local/share/workspacer/build-stamp || { echo "build stamp missing $key" >&2; exit 1; }
done
if [ -d /data ] && [ -n "$(find /data -mindepth 1 -print -quit)" ]; then
  printf '%s\n' 'image must not bake state into the mounted volume path' >&2; exit 1
fi
for path in /root/.claude /root/.codex /root/.config/workspacer /root/.local/share/claudemon; do
  [ ! -e "$path" ] || { printf 'unexpected built-in account state: %s\n' "$path" >&2; exit 1; }
done
workspacer-rust serve --help >/dev/null

[ ! -e /usr/local/bin/desktop-host.cjs ]
