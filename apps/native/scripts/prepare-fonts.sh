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

# GPUI renders variable fonts at their default instance, so bold/semibold need
# static instances. FONT_PYTHON must have fontTools (pip install fonttools).
native_font_python="${FONT_PYTHON:-python3}"
"$native_font_python" -c 'import fontTools'
native_fonts="$native_font_root/assets/fonts"
native_instance() {
  "$native_font_python" -m fontTools.varLib.instancer "$native_fonts/$1-Variable.ttf" \
    "${@:3}" --static --update-name-table -o "$native_fonts/$1-$2.ttf"
}
native_instance Inter Medium wght=500 opsz=drop
native_instance Inter SemiBold wght=600 opsz=drop
native_instance Inter Bold wght=700 opsz=drop
native_instance JetBrainsMono Bold wght=700
# Instancer names Medium/SemiBold as separate legacy families; group every
# weight under "Inter" so GPUI matches them by weight.
"$native_font_python" - "$native_fonts" <<'PY'
import sys
from fontTools.ttLib import TTFont
for weight in ("Medium", "SemiBold"):
    path = f"{sys.argv[1]}/Inter-{weight}.ttf"
    font = TTFont(path)
    for record in font["name"].names:
        if record.nameID == 1:
            record.string = "Inter"
        elif record.nameID == 2:
            record.string = weight
    font.save(path)
PY
