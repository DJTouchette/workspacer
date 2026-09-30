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

The generated files are committed; the utility is not a build dependency.
