---
title: Native design system: palette, tokens, custom themes, gallery
tags: [native, gpui, theme, design-system, custom-themes]
related_paths:
  - "apps/native/src/appearance.rs"
  - "apps/native/src/ui/tokens.rs"
  - "apps/native/src/ui/chrome.rs"
  - "apps/native/src/ui/theme_editor.rs"
  - "apps/native/src/ui/design_system.rs"
  - "apps/native/DESIGN_SYSTEM.md"
owner: Damien Touchette
last_reviewed: 2026-11-06
---

# Native design system

## Overview
The GPUI client's look has three layers: the `Palette` (colors + surface radii, built-in
`Appearance` or a user `CustomTheme`), theme-independent tokens in `src/ui/tokens.rs`
(`scale`, `icon_size`, `radius`, `control`, `alpha`), and shared components in
`chrome.rs` / `Workspace::button*`. `apps/native/DESIGN_SYSTEM.md` is the human guide;
`Screen::DesignSystem` (`g y`, Settings → Appearance) renders it live.

## Gotchas
- Read colors with `Workspace::palette()`; `self.appearance.palette()` ignores the
  active custom theme. With a custom theme active, `self.appearance` is the built-in it
  *reads like* (syntax colors, light/dark mode), not what is drawn.
- `tokens_are_used_instead_of_literals` scans `src/ui` source text: numeric
  `text_size(px(`, `rounded(px(`, `tonal(p, ` and icon/svg `.size(px(` fail the build.
  Layout box sizes (`div().size(px(28.))`) and `#[cfg(test)]` tails are not checked.
- Custom themes persist only changed colors in `native-custom-themes.json`; the
  editor's hex fields resync through `InputEvent::Change`, and an unchanged parse is a
  no-op, which is what prevents a feedback loop.
- `smoke.py` drives screens with two-key `g <x>` chords; very large software-rendered
  windows can miss the chord timeout. Use `--wheel N` at normal size to reach lower content.
