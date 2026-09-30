#!/usr/bin/env bash
set -euo pipefail
native_font_root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
native_font_source="$native_font_root/../desktop/src/renderer/src/assets/fonts"
native_font_tmp="$(mktemp -d)"
trap 'rm -rf "$native_font_tmp"' EXIT
command -v woff2_decompress >/dev/null
mkdir -p "$native_font_root/assets/fonts"
for native_font_name in Inter-Variable JetBrainsMono-Variable; do
  cp "$native_font_source/$native_font_name.woff2" "$native_font_tmp/"
  woff2_decompress "$native_font_tmp/$native_font_name.woff2"
  cp "$native_font_tmp/$native_font_name.ttf" "$native_font_root/assets/fonts/"
done
