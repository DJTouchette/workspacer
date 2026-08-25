#!/usr/bin/env bash
#
# verify-image.sh — assert that a workspacer node image is still a valid one.
#
# Runs INSIDE a `docker build`, as the last RUN of every layer that extends the
# workspacer node base image:
#
#     RUN /usr/local/lib/wks/verify-image.sh
#
# It exists because of one rule that a downstream Dockerfile cannot see and
# will not guess:
#
#   ***  $HOME IS A MOUNT POINT. NOTHING MAY BE INSTALLED INTO IT.  ***
#
# On the node, $HOME is /data/home, which lives on a Fly volume mounted at
# /data. The volume is mounted AFTER the image is built and it SHADOWS whatever
# the image put there. So `bundle install --path ~/.gems`, `npm i -g` with a
# HOME-relative prefix, `curl … | bash` installers that default to ~/.bun,
# ~/.cargo, ~/.rustup, ~/.deno, ~/.local/bin — every one of those produces an
# image that builds green, passes a `docker run` smoke test, and then loses the
# tool the first time the real machine boots with its volume attached.
#
# That failure is silent and it is a day to debug. This script converts it into
# a build error with a filename in it. That is the entire point.
#
# Everything must go to /usr/local instead. The base already puts /usr/local/bin
# and /usr/local/go/bin on PATH, so a downstream layer never needs to touch
# PATH either.
#
# The second half of the script checks that the layer above did not break the
# base's own contract — the three daemons, the entrypoint, the wks user and its
# uid/gid. Those are what `entrypoint.sh` and `fly.toml` assume, and a
# downstream `USER`, `useradd` or `ENV HOME=` silently invalidates them.
#
# Exit 0 = the image is still a workspacer node. Non-zero = it is not, and the
# message says which rule broke.

set -euo pipefail

fail_count=0
warn_count=0

err()  { printf '  ✗ %s\n' "$*" >&2; fail_count=$((fail_count + 1)); }
# Continuation lines for a single error — advice, not a second finding, so they
# must not inflate the count.
errmore() { printf '    %s\n' "$*" >&2; }
warn() { printf '  ! %s\n' "$*" >&2; warn_count=$((warn_count + 1)); }
ok()   { printf '  ✓ %s\n' "$*"; }
hdr()  { printf '\n%s\n' "$*"; }

# The uid/gid the base creates and fly.toml/entrypoint.sh assume.
WKS_UID_EXPECTED="${WKS_UID:-10001}"
WKS_GID_EXPECTED="${WKS_GID:-10001}"
WKS_USER_EXPECTED="${WKS_USER:-wks}"

# Where the volume will mount, and the $HOME under it. Nothing in the image may
# live here: it is all shadowed at boot.
DATA_DIR="${WKS_DATA:-/data}"
HOME_DIR="${WKS_HOME:-${DATA_DIR}/home}"

printf 'verify-image.sh — checking the workspacer node image contract\n'

# ---------------------------------------------------------------------------
hdr 'The volume mount point must be empty in the image'
# ---------------------------------------------------------------------------
# /data is where the volume lands. Anything the image writes under it is
# invisible from the moment the machine boots. bootstrap.sh creates the real
# contents at runtime, as the wks user, on the volume.
if [ -e "$DATA_DIR" ]; then
  strays="$(find "$DATA_DIR" -mindepth 1 -maxdepth 3 2>/dev/null | head -20 || true)"
  if [ -n "$strays" ]; then
    err "$DATA_DIR is not empty in the image. The volume mounts over it at boot,"
    errmore "so every one of these is discarded on the first wake:"
    printf '%s\n' "$strays" | sed 's/^/      /' >&2
    errmore "Install to /usr/local instead. If you need a directory to exist at"
    errmore "runtime, create it in bootstrap.sh — not in a Dockerfile."
  else
    ok "$DATA_DIR exists but is empty"
  fi
else
  ok "$DATA_DIR does not exist in the image"
fi

if [ -e "$HOME_DIR" ]; then
  err "$HOME_DIR exists in the image. That is the node's \$HOME and it lives on"
  errmore "the volume — the image copy is shadowed at boot and silently lost."
fi

# ---------------------------------------------------------------------------
hdr 'No toolchain may be installed into a home directory'
# ---------------------------------------------------------------------------
# During `docker build` the user is root, so a HOME-relative installer lands in
# /root rather than /data/home. Same bug, different path: /root is on the
# ephemeral rootfs and the wks user cannot read it anyway.
#
# These are the directories that installers create when they are allowed to
# default to $HOME. Each one means "this tool will vanish at boot".
home_toolchains='
.bun
.cargo
.rustup
.deno
.nvm
.rbenv
.rvm
.pyenv
.asdf
.mise
.gem
.bundle
.local/bin
.local/share/gem
.local/share/mise
.npm-global
.yarn
.pnpm-store
.volta
.sdkman
.opam
.stack
.ghcup
.dotnet
node_modules
'

home_fails_before="$fail_count"
for home_root in /root /home/*; do
  [ -d "$home_root" ] || continue
  for tc in $home_toolchains; do
    if [ -e "$home_root/$tc" ]; then
      err "$home_root/$tc exists — a toolchain was installed into \$HOME."
      errmore "Reinstall it under /usr/local. See the header of this script."
    fi
  done
done

# A per-user home for wks is a different flavour of the same mistake: the base
# deliberately creates wks with -M and $HOME on the volume.
if [ -d /home/"$WKS_USER_EXPECTED" ]; then
  err "/home/$WKS_USER_EXPECTED exists. The wks user's \$HOME is $HOME_DIR, on the"
  errmore "volume. A second home on the rootfs is not the one anything will use."
fi

if [ "$fail_count" -eq "$home_fails_before" ]; then
  ok 'no toolchains under any home directory'
fi

# Build caches under /root are not a correctness bug — nothing reads them at
# runtime — but they are dead weight in every layer below them.
for cache in /root/.npm /root/.cache /root/.bundle/cache; do
  [ -e "$cache" ] && warn "$cache is dead weight in the image; append a cleanup to the RUN that made it"
done

# ---------------------------------------------------------------------------
hdr 'No stateful ENV may be baked into the image'
# ---------------------------------------------------------------------------
# HOME, XDG_*, GOPATH and the caches are set by entrypoint.sh and fly.toml,
# never by a Dockerfile ENV — a Dockerfile ENV also applies during `docker
# build`, where /data does not exist, so it poisons the build it is trying to
# configure. PATH is the documented exception.
#
# `docker build` cannot introspect its own ENV table, but it can catch the case
# that actually matters: a value pointing into the volume, which can only have
# come from a Dockerfile.
for var in HOME XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_CACHE_HOME \
           GOPATH GOMODCACHE GOCACHE BUNDLE_PATH BUN_INSTALL_CACHE_DIR npm_config_cache; do
  val="$(printenv "$var" 2>/dev/null || true)"
  case "$val" in
    "$DATA_DIR"|"$DATA_DIR"/*)
      err "ENV $var=$val is baked into the image. $DATA_DIR does not exist during"
      errmore "a build, so this breaks the build; and at runtime entrypoint.sh and"
      errmore "fly.toml set it anyway. Delete the ENV."
      ;;
  esac
done
[ "${HOME:-/root}" = "$HOME_DIR" ] && err "ENV HOME=$HOME_DIR is baked into the image. Remove it; the entrypoint sets HOME."

# ---------------------------------------------------------------------------
hdr 'The base image contract is intact'
# ---------------------------------------------------------------------------
for bin in brain workspacer claudemon tailscaled tailscale claude tini; do
  if command -v "$bin" >/dev/null 2>&1; then
    ok "$bin on PATH ($(command -v "$bin"))"
  else
    err "$bin is missing or fell off PATH — a downstream layer removed it or overrode PATH"
  fi
done

for f in /usr/local/lib/wks/entrypoint.sh /usr/local/lib/wks/bootstrap.sh; do
  if [ -x "$f" ]; then ok "$f is present and executable"; else err "$f is missing or not executable"; fi
done

user_fails_before="$fail_count"
if id "$WKS_USER_EXPECTED" >/dev/null 2>&1; then
  actual_uid="$(id -u "$WKS_USER_EXPECTED")"
  actual_gid="$(id -g "$WKS_USER_EXPECTED")"
  actual_home="$(getent passwd "$WKS_USER_EXPECTED" | cut -d: -f6)"
  [ "$actual_uid" = "$WKS_UID_EXPECTED" ] || err "user $WKS_USER_EXPECTED has uid $actual_uid, expected $WKS_UID_EXPECTED (the volume's files are chowned to it)"
  [ "$actual_gid" = "$WKS_GID_EXPECTED" ] || err "user $WKS_USER_EXPECTED has gid $actual_gid, expected $WKS_GID_EXPECTED"
  [ "$actual_home" = "$HOME_DIR" ] || err "user $WKS_USER_EXPECTED has home $actual_home, expected $HOME_DIR"
  [ "$actual_uid" = "0" ] && err "the agent user must not be root: Claude Code refuses --dangerously-skip-permissions at euid 0"
  [ "$fail_count" -eq "$user_fails_before" ] && ok "user $WKS_USER_EXPECTED is $actual_uid:$actual_gid with home $HOME_DIR"
else
  err "user $WKS_USER_EXPECTED does not exist — the entrypoint drops privilege to it"
fi

# The entrypoint runs as root and drops privilege itself. A downstream `USER
# wks` would leave it unable to run tailscaled or chown the volume.
current_uid="$(id -u)"
[ "$current_uid" = "0" ] || err "this layer is building as uid $current_uid, not root: do not put USER in a downstream Dockerfile, the entrypoint drops privilege with setpriv"

# ---------------------------------------------------------------------------
printf '\n'
if [ "$warn_count" -gt 0 ]; then
  printf '%d warning(s)\n' "$warn_count" >&2
fi
if [ "$fail_count" -gt 0 ]; then
  printf 'verify-image.sh: FAILED with %d error(s). The image above is not a usable workspacer node.\n' "$fail_count" >&2
  exit 1
fi
printf 'verify-image.sh: OK — image satisfies the workspacer node contract.\n'
