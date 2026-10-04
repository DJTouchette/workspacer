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
native_instance JetBrainsMono Regular wght=400
# Italics: the desktop has no italic asset (browsers slant regular Inter), but
# GPUI never synthesizes one, so cut static faces from the matching Inter 4.1
# release (same version and OFL license as Inter-Variable).
native_inter_zip="$native_font_tmp/inter.zip"
curl -fsSL -o "$native_inter_zip" https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
echo "9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e  $native_inter_zip" | sha256sum -c -
unzip -q -o "$native_inter_zip" InterVariable-Italic.ttf -d "$native_font_tmp"
native_italic() {
  "$native_font_python" -m fontTools.varLib.instancer "$native_font_tmp/InterVariable-Italic.ttf" \
    "wght=$2" opsz=drop --static --update-name-table -o "$native_fonts/Inter-$1.ttf"
}
native_italic Italic 400
native_italic BoldItalic 700
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
# The italic instances keep "Inter Variable" as their typographic family;
# rename every family/style record so GPUI files them under Inter too.
for file, style in (("Italic", "Italic"), ("BoldItalic", "Bold Italic")):
    path = f"{sys.argv[1]}/Inter-{file}.ttf"
    font = TTFont(path)
    names = font["name"]
    for name_id, value in ((1, "Inter"), (2, style), (4, f"Inter {style}"),
                           (6, f"Inter-{file}"), (16, "Inter"), (17, style)):
        names.setName(value, name_id, 3, 1, 0x409)
    names.removeNames(platformID=1)
    font.save(path)
# Inline code: desktop draws it at 0.9em, but GPUI shapes a line at one size.
# "JetBrains Mono Inline" scales the outlines and advances to 90% inside the
# same em, keeping the line metrics, so it sits on the prose baseline smaller.
from fontTools.ttLib.scaleUpem import scale_upem
fam = "JetBrains Mono Inline"
for file in ("Regular", "Bold"):
    font = TTFont(f"{sys.argv[1]}/JetBrainsMono-{file}.ttf")
    upem = font["head"].unitsPerEm
    keep = {(t, a): getattr(font[t], a) for t, a in (
        ("hhea", "ascent"), ("hhea", "descent"), ("hhea", "lineGap"),
        ("OS/2", "sTypoAscender"), ("OS/2", "sTypoDescender"), ("OS/2", "sTypoLineGap"),
        ("OS/2", "usWinAscent"), ("OS/2", "usWinDescent"))}
    scale_upem(font, round(upem * 0.9))
    font["head"].unitsPerEm = upem
    for (table, attr), value in keep.items():
        setattr(font[table], attr, value)
    names = font["name"]
    for name_id, value in ((1, fam), (2, file), (4, f"{fam} {file}"),
                           (6, f"JetBrainsMonoInline-{file}"), (16, fam), (17, file)):
        names.setName(value, name_id, 3, 1, 0x409)
    names.removeNames(platformID=1)
    font.save(f"{sys.argv[1]}/JetBrainsMonoInline-{file}.ttf")
PY
# Only the scaled inline faces ship the Regular instance.
rm "$native_fonts/JetBrainsMono-Regular.ttf"
