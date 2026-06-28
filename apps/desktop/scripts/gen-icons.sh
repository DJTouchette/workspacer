#!/usr/bin/env bash
# Regenerate all raster app-icon / favicon assets from the master SVGs.
#
#   build/icon.svg        — primary { ▮ } mark  (used ≥48px)
#   build/icon-small.svg  — small-size variant  (used ≤32px, wider gaps)
#
# Outputs:
#   build/icon.png                    1024² — electron-builder derives .icns/.ico from this
#   build/icon.ico                    multi-res — Windows runtime BrowserWindow icon
#   src/renderer/public/icon.png      256²  — main-process window + notification icon
#   src/renderer/public/favicon-16.png / favicon-32.png — <head> favicon tiers
#
# Requires: rsvg-convert (librsvg), magick (ImageMagick), JetBrains Mono Bold installed.
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD=build
PUBLIC=src/renderer/public
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$PUBLIC"

render() { # <svg> <size> <out>
  rsvg-convert -w "$2" -h "$2" "$1" -o "$3"
}

# Primary mark — larger tiers.
for s in 48 64 128 256 512 1024; do
  render "$BUILD/icon.svg" "$s" "$TMP/icon-$s.png"
done
# Small variant — favicon tiers that downscale poorly with the detailed mark.
for s in 16 24 32; do
  render "$BUILD/icon-small.svg" "$s" "$TMP/icon-$s.png"
done

# electron-builder master (auto-derives .icns / .ico at package time).
cp "$TMP/icon-1024.png" "$BUILD/icon.png"

# Windows runtime icon — multi-resolution .ico.
magick "$TMP/icon-16.png" "$TMP/icon-24.png" "$TMP/icon-32.png" \
       "$TMP/icon-48.png" "$TMP/icon-64.png" "$TMP/icon-128.png" \
       "$TMP/icon-256.png" "$BUILD/icon.ico"

# Renderer-served assets: runtime window/notification icon + favicon tiers.
cp "$TMP/icon-256.png" "$PUBLIC/icon.png"
cp "$TMP/icon-16.png"  "$PUBLIC/favicon-16.png"
cp "$TMP/icon-32.png"  "$PUBLIC/favicon-32.png"

echo "Generated:"
echo "  $BUILD/icon.png $BUILD/icon.ico"
echo "  $PUBLIC/icon.png $PUBLIC/favicon-16.png $PUBLIC/favicon-32.png"
