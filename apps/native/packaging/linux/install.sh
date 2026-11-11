#!/bin/sh
# Install Workspacer Native from this unpacked archive into PREFIX
# (default ~/.local). Records every file it writes so uninstall.sh removes
# exactly those and nothing a person put there.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${PREFIX:-$HOME/.local}
lib=$prefix/lib/workspacer-native
manifest=$lib/installed-files

[ -x "$here/lib/workspacer-native/wks-native" ] || {
    echo "install.sh must run from the unpacked archive" >&2
    exit 1
}

put() { # put <mode> <source> <destination>
    mkdir -p "$(dirname -- "$3")"
    install -m "$1" "$2" "$3"
    printf '%s\n' "$3" >>"$manifest.new"
}

mkdir -p "$lib"
: >"$manifest.new"
put 755 "$here/lib/workspacer-native/wks-native" "$lib/wks-native"
put 644 "$here/lib/workspacer-native/build-stamp.json" "$lib/build-stamp.json"
put 755 "$here/uninstall.sh" "$lib/uninstall.sh"
put 644 "$here/README.txt" "$prefix/share/doc/workspacer-native/README.txt"
for license in "$here"/share/licenses/workspacer-native/*; do
    put 644 "$license" "$prefix/share/licenses/workspacer-native/$(basename -- "$license")"
done
put 644 "$here/share/icons/hicolor/scalable/apps/workspacer-native.svg" \
    "$prefix/share/icons/hicolor/scalable/apps/workspacer-native.svg"

# The launcher names the installed binary by full path: ~/.local/bin is not
# on every desktop session's PATH.
desktop=$prefix/share/applications/workspacer-native.desktop
mkdir -p "$(dirname -- "$desktop")"
sed "s|^Exec=wks-native|Exec=\"$lib/wks-native\"|" \
    "$here/share/applications/workspacer-native.desktop" >"$desktop"
chmod 644 "$desktop"
printf '%s\n' "$desktop" >>"$manifest.new"

mkdir -p "$prefix/bin"
for link in wks-native:wks-native wks-native-uninstall:uninstall.sh; do
    ln -sf "$lib/${link#*:}" "$prefix/bin/${link%%:*}"
    printf '%s\n' "$prefix/bin/${link%%:*}" >>"$manifest.new"
done

mv "$manifest.new" "$manifest"
command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true

echo "Installed Workspacer Native to $prefix"
case ":$PATH:" in
*":$prefix/bin:"*) ;;
*) echo "Note: $prefix/bin is not on your PATH; start it from your launcher, or run $prefix/bin/wks-native" ;;
esac
