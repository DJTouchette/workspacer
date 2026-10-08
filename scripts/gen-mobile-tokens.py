#!/usr/bin/env python3
"""Generate the /m-next design tokens from the native client's palettes.

`apps/native/src/appearance.rs` is the single source of truth for every theme
the native client ships. The mobile web client consumes the same colours as
CSS custom properties, so this renders:

  services/hub-rs/assets/web/m-next/tokens.css   [data-theme="<slug>"] blocks,
                                                  one --wks-<field> per Palette
                                                  field, plus the chrome scale
  services/hub-rs/assets/web/m-next/js/themes.js the theme list (slug, label,
                                                  dark) for the Settings picker

Code highlighting colours come from `apps/native/src/ui/syntax.rs` (the
per-appearance `Syntax` table) as --wks-syn-<capture> in the same blocks.

Slugs are the serde (kebab-case) names native persists, so a phone's choice
reads the same as a desktop's. The parser is strict: every enum variant must
have a label, a palette block and every Palette field, or generation fails
rather than silently dropping a colour.

Usage:
  gen-mobile-tokens.py            write both files
  gen-mobile-tokens.py --check    exit 1 when the checked-in files drift
  gen-mobile-tokens.py --source <appearance.rs> --syntax <syntax.rs> --out <dir>
"""
import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "apps/native/src/appearance.rs")
SYNTAX = os.path.join(ROOT, "apps/native/src/ui/syntax.rs")
OUT = os.path.join(ROOT, "services/hub-rs/assets/web/m-next")


def fail(message):
    raise SystemExit(f"gen-mobile-tokens: {message}")


def kebab(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()


def block(src, start):
    """The text of the `{ … }` block whose opening brace follows `start`."""
    i = src.index("{", start)
    depth = 0
    for j in range(i, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return src[i + 1 : j]
    fail("unbalanced braces in appearance.rs")


def parse(src):
    enum = re.search(r"pub enum Appearance\s*\{", src)
    if not enum:
        fail("no `pub enum Appearance` in the source")
    body = re.sub(r"//[^\n]*", "", block(src, enum.start()))
    body = re.sub(r"#\[[^\]]*\]", "", body)
    variants = [v.strip() for v in body.split(",") if v.strip()]
    if not variants or not all(re.fullmatch(r"[A-Z]\w*", v) for v in variants):
        fail(f"could not read the Appearance variants: {variants!r}")

    palette = re.search(r"pub struct Palette\s*\{", src)
    if not palette:
        fail("no `pub struct Palette` in the source")
    fields = re.findall(r"pub (\w+):\s*(u32|f32)", block(src, palette.start()))
    if not fields:
        fail("could not read the Palette fields")

    labels = dict(re.findall(r"Self::(\w+)\s*=>\s*\"([^\"]+)\"", block(src, src.index("pub fn label(self)"))))
    light = re.findall(r"Self::(\w+)", block(src, src.index("pub fn is_dark(self)")))

    palettes = {}
    pal_body = block(src, src.index("pub fn palette(self)"))
    for name, start in [(m.group(1), m.end() - 1) for m in re.finditer(r"Self::(\w+)\s*=>\s*Palette\s*\{", pal_body)]:
        values = dict(re.findall(r"(\w+):\s*([0-9a-fA-Fx_.]+)\s*,", block(pal_body, start)))
        palettes[name] = values

    themes = []
    for variant in variants:
        if variant not in labels:
            fail(f"{variant} has no label()")
        if variant not in palettes:
            fail(f"{variant} has no palette() block")
        values = palettes[variant]
        missing = [f for f, _ in fields if f not in values]
        if missing:
            fail(f"{variant} palette is missing {missing}")
        themes.append({
            "variant": variant,
            "slug": kebab(variant),
            "label": labels[variant],
            "dark": variant not in light,
            "values": [(f, kind, values[f].replace("_", "")) for f, kind in fields],
        })
    return themes


def parse_syntax(src, themes):
    """Attach each theme's code colours (`Syntax` consts, mapped per variant)."""
    struct = re.search(r"struct Syntax\s*\{", src)
    if not struct:
        fail("no `struct Syntax` in syntax.rs")
    fields = re.findall(r"(\w+):\s*&'static str", block(src, struct.start()))
    consts = {}
    for m in re.finditer(r"const (\w+): Syntax = Syntax\s*\{", src):
        consts[m.group(1)] = dict(re.findall(r"(\w+):\s*\"(#[0-9a-fA-F]{6})\"", block(src, m.end() - 1)))
    mapping = dict(re.findall(r"Appearance::(\w+)\s*=>\s*&(\w+)", src))
    for t in themes:
        name = mapping.get(t["variant"])
        if not name or name not in consts:
            fail(f"{t['variant']} has no syntax colours")
        missing = [f for f in fields if f not in consts[name]]
        if missing:
            fail(f"{name} syntax is missing {missing}")
        t["syntax"] = [(f, consts[name][f].lower()) for f in fields]


def css_value(field, kind, raw):
    if kind == "u32":
        hexv = raw[2:] if raw.lower().startswith("0x") else fail(f"{field}: {raw} is not hex")
        if len(hexv) == 8:  # 0xRRGGBBAA
            r, g, b, a = (int(hexv[i : i + 2], 16) for i in (0, 2, 4, 6))
            return f"rgba({r}, {g}, {b}, {a / 255:.3f})"
        return "#" + hexv.lower().zfill(6)
    number = float(raw.rstrip("."))
    return f"{number:g}px" if field.endswith("radius") else f"{number:g}"


def render(themes):
    head = (
        "/* GENERATED from apps/native/src/appearance.rs by scripts/gen-mobile-tokens.py.\n"
        "   Do not edit: change the native palette and regenerate (make check-hub-rust-assets\n"
        "   fails when this file drifts from it). */"
    )
    blocks = [head]
    for t in themes:
        lines = [f"  --wks-{f.replace('_', '-')}: {css_value(f, k, v)};" for f, k, v in t["values"]]
        lines += [f"  --wks-syn-{f}: {v};" for f, v in t.get("syntax", [])]
        lines.append(f"  color-scheme: {'dark' if t['dark'] else 'light'};")
        blocks.append(f'[data-theme="{t["slug"]}"] {{\n' + "\n".join(lines) + "\n}")
    # The chrome type scale (ui/chrome.rs `scale`) and fixed shapes every theme
    # shares. Anthropic clay is chrome::CLAUDE_CLAY.
    blocks.append(
        ":root {\n"
        '  --wks-font-ui: "Inter", system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;\n'
        '  --wks-font-mono: "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, monospace;\n'
        "  --wks-title: 22px; --wks-title-short: 18px; --wks-heading: 15px; --wks-body: 13px;\n"
        "  --wks-meta: 12px; --wks-caption: 11px; --wks-overline: 10px;\n"
        "  --wks-island-radius: 20px; --wks-island-height: 40px; --wks-status-dot: 6px;\n"
        "  --wks-claude-clay: #d97757;\n"
        "}"
    )
    css = "\n\n".join(blocks) + "\n"
    rows = ",\n".join(
        f"  {{ slug: '{t['slug']}', label: '{t['label']}', dark: {'true' if t['dark'] else 'false'}, "
        f"chat: '{css_value('chat', 'u32', dict((f, v) for f, _, v in t['values'])['chat'])}' }}"
        for t in themes
    )
    js = (
        "// GENERATED from apps/native/src/appearance.rs by scripts/gen-mobile-tokens.py.\n"
        "// Do not edit. Slugs are native's persisted (serde kebab-case) theme names.\n"
        f"export const THEMES = [\n{rows},\n];\n"
    )
    return {"tokens.css": css, os.path.join("js", "themes.js"): js}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--source", default=SOURCE)
    ap.add_argument("--syntax", default=SYNTAX)
    ap.add_argument("--out", default=OUT)
    args = ap.parse_args()
    with open(args.source, encoding="utf-8") as f:
        themes = parse(f.read())
    with open(args.syntax, encoding="utf-8") as f:
        parse_syntax(f.read(), themes)
    files = render(themes)
    drift = []
    for name, text in files.items():
        path = os.path.join(args.out, name)
        if args.check:
            try:
                with open(path, encoding="utf-8") as f:
                    current = f.read()
            except FileNotFoundError:
                current = None
            if current != text:
                drift.append(os.path.relpath(path, ROOT))
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
    if drift:
        print(
            "gen-mobile-tokens: drift from the native palettes in "
            + ", ".join(drift)
            + "\n  run: python3 scripts/gen-mobile-tokens.py",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
