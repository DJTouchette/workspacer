#!/usr/bin/env bash
# Package the native client for Linux as a relocatable tar.gz with a
# per-user installer. Usage: package-linux.sh [--no-build] [--version V]
#
# The version defaults to apps/desktop/package.json's, the release version
# the Windows installer is stamped with too; the app reads it back from
# build-stamp.json beside the binary (features::installed_version).
set -euo pipefail
native=$(cd -- "$(dirname -- "$0")/.." && pwd)
root=$(cd -- "$native/../.." && pwd)
build=1
version=
while [ $# -gt 0 ]; do
    case "$1" in
    --no-build) build=0 ;;
    --version) version=$2; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done
if [ -z "$version" ]; then
    version=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' "$root/apps/desktop/package.json" | head -1)
fi
[ -n "$version" ] || { echo "could not read the release version" >&2; exit 1; }
case "$(uname -m)" in
x86_64) arch=x64 ;;
aarch64) arch=arm64 ;;
*) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [ "$build" = 1 ]; then
    (cd "$native" && cargo build --locked --release --bin wks-native)
fi
binary=$native/target/release/wks-native
[ -x "$binary" ] || { echo "missing $binary (build it, or drop --no-build)" >&2; exit 1; }

name=workspacer-native-$version-linux-$arch
out=$native/target/package
stage=$out/$name
rm -rf "$stage" "$out/$name.tar.gz" "$out/$name.tar.gz.sha256"
mkdir -p "$stage/lib/workspacer-native" "$stage/share/applications" \
    "$stage/share/icons/hicolor/scalable/apps" "$stage/share/licenses/workspacer-native"

install -m 755 "$binary" "$stage/lib/workspacer-native/wks-native"
commit=$(git -C "$root" rev-parse HEAD 2>/dev/null || true)
if [ -n "$commit" ]; then commit="\"$commit\""; else commit=null; fi
cat >"$stage/lib/workspacer-native/build-stamp.json" <<STAMP
{
  "component": "native",
  "version": "$version",
  "commit": $commit,
  "platform": "linux-$arch",
  "backend": "rust"
}
STAMP
install -m 644 "$native/packaging/linux/workspacer-native.desktop" "$stage/share/applications/"
install -m 644 "$root/apps/desktop/build/icon.svg" \
    "$stage/share/icons/hicolor/scalable/apps/workspacer-native.svg"
install -m 644 "$root/LICENSE" "$stage/share/licenses/workspacer-native/LICENSE"
install -m 644 "$native/assets/fonts/Inter-LICENSE.txt" "$native/assets/fonts/JetBrainsMono-LICENSE.txt" \
    "$stage/share/licenses/workspacer-native/"
install -m 644 "$native/packaging/linux/README.txt" "$stage/README.txt"
install -m 755 "$native/packaging/linux/install.sh" "$native/packaging/linux/uninstall.sh" "$stage/"

# Reproducible member order and ownership; the mtime is the commit's.
stamp=$(git -C "$root" log -1 --format=%cI 2>/dev/null || date -u +%Y-%m-%dT%H:%M:%SZ)
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="$stamp" \
    -C "$out" -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
rm -rf "$stage"
echo "$out/$name.tar.gz"
