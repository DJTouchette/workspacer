# Native fonts

The native client embeds Inter for interface text and JetBrains Mono for code.
They work without an OS font installation or runtime download. Both are licensed
under the SIL Open Font License; the full licenses are included here.
They are also embedded in the executable and accessible with
`wks-native --font-licenses`.

- [Inter](https://rsms.me/inter/): `Inter-LICENSE.txt`
- [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono): `JetBrainsMono-LICENSE.txt`

The TTF files are losslessly decompressed from the existing desktop WOFF2 assets
in `apps/desktop/src/renderer/src/assets/fonts`. GPUI needs SFNT font data.
To regenerate them, install the `woff2_decompress` utility and run:

```sh
bash apps/native/scripts/prepare-fonts.sh
```

GPUI renders a variable font only at its default instance, so the script also
derives static Inter Medium/SemiBold/Bold and JetBrains Mono Bold instances with
fontTools; without them Markdown bold and semibold headings render at regular
weight. Point `FONT_PYTHON` at an interpreter with `fonttools` installed.

The desktop has no italic asset (browsers slant regular Inter); GPUI does not
synthesize italics, so Inter Italic and Bold Italic are static instances of
`InterVariable-Italic.ttf` from the official Inter 4.1 release, the same
version and license as the variable font. The script downloads that release
(checksum-pinned), which also needs `curl` and `unzip`.

The generated files are committed; neither utility is a build dependency.

`JetBrainsMonoInline-{Regular,Bold}.ttf` ("JetBrains Mono Inline") are JetBrains
Mono with outlines and advances scaled to 90% inside the same em and line
metrics. Desktop inline code is 0.9em; GPUI shapes a whole line at one font
size, so inline code switches to this family instead while the code font is the
bundled JetBrains Mono. It is hidden from the font pickers.
