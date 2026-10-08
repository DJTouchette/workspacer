# /m-next fonts

Inter (interface) and JetBrains Mono (code), served by the hub so the phone
never reaches a third-party font host. Both are SIL Open Font License 1.1;
the full licenses sit beside the files.

They are Latin subsets of the desktop's variable WOFF2 files
(`apps/desktop/src/renderer/src/assets/fonts`), weight axis kept. Regenerate
with fontTools (`pip install fonttools brotli`):

```sh
U="U+0000-00FF,U+0100-017F,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0300-0308,U+2000-206F,U+2070-209F,U+20AC,U+2122,U+2190-21FF,U+2212,U+2215,U+2248,U+2260,U+2264-2265,U+2300-23FF,U+2500-257F,U+25A0-25FF,U+2600-26FF,U+2700-27BF,U+FEFF,U+FFFD"
for f in Inter-Variable JetBrainsMono-Variable; do
  pyftsubset apps/desktop/src/renderer/src/assets/fonts/$f.woff2 --unicodes="$U" \
    --layout-features='*' --flavor=woff2 --output-file=services/hub-rs/assets/web/m-next/fonts/$f.woff2
done
```
