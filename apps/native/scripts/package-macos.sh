#!/usr/bin/env bash
# Package the native client for macOS as Workspacer.app inside a .dmg.
# Usage: package-macos.sh [--no-build] [--version V]
#
# The app is ad-hoc signed, not notarized: a downloaded copy needs its
# quarantine flag cleared once (xattr -cr /Applications/Workspacer.app).
# Launched from Finder it owns its local engine (see macos_bundle_launch in
# src/main.rs); the version is read back from build-stamp.json beside the
# binary (in Contents/Resources: codesign treats Contents/MacOS as code).
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
    version=$(tr -d '[:space:]' <"$root/VERSION")
fi
[ -n "$version" ] || { echo "could not read the release version" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || { echo "package on macOS" >&2; exit 1; }
case "$(uname -m)" in
arm64) arch=arm64 ;;
x86_64) arch=x64 ;;
*) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
# CFBundleShortVersionString takes digits only; the full version (with any
# -nightly stamp) stays in CFBundleVersion's companion, the build stamp.
short_version=${version%%-*}

if [ "$build" = 1 ]; then
    (cd "$native" && cargo build --locked --release --bin wks-native)
fi
binary=$native/target/release/wks-native
[ -x "$binary" ] || { echo "missing $binary (build it, or drop --no-build)" >&2; exit 1; }

out=$native/target/package
name=Workspacer-$version-macos-$arch
work=$out/macos-stage
app=$work/Workspacer.app
rm -rf "$work" "$out/$name.dmg" "$out/$name.dmg.sha256"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

install -m 755 "$binary" "$app/Contents/MacOS/wks-native"
commit=$(git -C "$root" rev-parse HEAD 2>/dev/null || true)
if [ -n "$commit" ]; then commit="\"$commit\""; else commit=null; fi
cat >"$app/Contents/Resources/build-stamp.json" <<STAMP
{
  "component": "native",
  "version": "$version",
  "commit": $commit,
  "platform": "darwin-$arch",
  "backend": "rust"
}
STAMP

# Icon: every size iconutil expects, from the 1024px PNG.
iconset=$work/Workspacer.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z $size $size "$native/packaging/icons/icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$native/packaging/icons/icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Workspacer.icns"
rm -rf "$iconset"

cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Workspacer</string>
  <key>CFBundleDisplayName</key><string>Workspacer</string>
  <key>CFBundleIdentifier</key><string>dev.workspacer.native</string>
  <key>CFBundleExecutable</key><string>wks-native</string>
  <key>CFBundleIconFile</key><string>Workspacer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$short_version</string>
  <key>CFBundleVersion</key><string>$short_version</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null

mkdir -p "$app/Contents/Resources/licenses"
install -m 644 "$root/LICENSE" "$native/assets/fonts/Inter-LICENSE.txt" \
    "$native/assets/fonts/JetBrainsMono-LICENSE.txt" "$app/Contents/Resources/licenses/"

# Ad-hoc signature: Apple Silicon refuses to run unsigned code at all.
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app"

ln -s /Applications "$work/Applications"
install -m 644 "$native/packaging/macos/README.txt" "$work/README.txt"
hdiutil create -volname Workspacer -srcfolder "$work" -fs HFS+ -format UDZO -ov "$out/$name.dmg" >/dev/null
(cd "$out" && shasum -a 256 "$name.dmg" >"$name.dmg.sha256")
rm -rf "$work"
echo "$out/$name.dmg"
