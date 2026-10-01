---
title: Vendored GPUI button hover used a hardcoded red foreground
date: 2026-10-01
confidence: high
related_paths:
  - vendor/gpui-component/src/button/button.rs
  - vendor/gpui-component/src/styled.rs
  - apps/native/src/appearance.rs
promoted: false
---

# Vendored GPUI button hover used a hardcoded red foreground

## Observation
gpui-component Button::render computed hover_style from its variant and theme, but applied crate::red_400() as text_color. It now uses hover_style.fg. Native configure_theme supplies its primary_hover, secondary_hover and accent ring tokens, and the native palettes include shared control/panel/composer radii. FocusableExt ring alpha increased from 0.2 to 0.8 for visible focus. WORKSPACER-PATCHES.md records both vendor changes for upgrades.

## Impact
Use resolved variant foregrounds and native semantic tokens when upgrading or styling component controls.
