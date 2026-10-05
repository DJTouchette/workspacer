---
title: Native chat selection was painted over the glyphs
date: 2026-10-05
confidence: high
suggested_doc: theme-system
related_paths:
  - vendor/gpui-component/src/text/inline.rs
  - apps/native/src/appearance.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native chat selection was painted over the glyphs

## Observation
gpui-component text/inline.rs Inline::paint painted styled text, then selection quads with cx.theme().selection on top. Native set theme.colors.selection to the opaque row-highlight color (p.selected), so selected chat/Markdown text (code blocks especially) became blank rectangles; Input (composer, file-viewer source) already painted selection first. Fixed by painting selection after inline-code backgrounds and before text (vendor patch) plus a translucent per-palette Palette.selection (0xRRGGBBAA).

## Impact
Any opaque theme.selection hides text in TextView; chat, docked/sheet Markdown viewer and the popped-out viewer all share Inline.

## Recommendation
Keep selection translucent and under glyphs; appearance tests composite selection over chat/code/user/surface and require 4.5:1 text.
