#!/bin/sh
# Thin wrapper so `docker run <image> status` / `token list` / `plugin dev` work
# the same as `serve` — everything after the image name is a workspacer command.
set -e

# Bind-mounted volumes arrive owned by the host uid. If that isn't ours we can't
# mint the pairing token or write the store, and the failure would surface much
# later as a confusing daemon error — say it plainly here instead.
for d in "$XDG_CONFIG_HOME/workspacer" "$HOME/.claude"; do
    [ -d "$d" ] || mkdir -p "$d" 2>/dev/null || true
    if [ -d "$d" ] && [ ! -w "$d" ]; then
        echo "workspacer: $d is not writable by uid $(id -u) —" \
             "run with --user \$(id -u):\$(id -g), or chown the mounted directory." >&2
    fi
done

# A HUB_TOKEN from the environment is picked up by `serve` as the flag default;
# with none set it mints one into the config volume and prints it in the banner.
exec workspacer "$@"
