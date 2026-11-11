#!/bin/sh
# Remove what install.sh wrote, by its own list. Sessions, settings and logs
# (under your data and config directories) are not touched.
set -eu
self=$(readlink -f -- "$0")
lib=$(dirname -- "$self")
manifest=$lib/installed-files
[ -f "$manifest" ] || {
    echo "No install record at $manifest; nothing removed." >&2
    exit 1
}
while IFS= read -r path; do
    [ -n "$path" ] || continue
    rm -f -- "$path"
    # Leave any directory that still holds something else.
    rmdir -p -- "$(dirname -- "$path")" 2>/dev/null || true
done <"$manifest"
rm -f -- "$manifest"
rmdir -p -- "$lib" 2>/dev/null || true
echo "Removed Workspacer Native. Your sessions and settings were kept."
