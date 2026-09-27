---
title: Native chrome and GPUI controls have separate theme sources
date: 2026-09-27
suggested_doc: theme-system
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/main.rs
promoted: false
---

# Native chrome and GPUI controls have separate theme sources

## Observation
apps/native/src/ui.rs colors its Div chrome directly, but Input and Markdown consume gpui_component::Theme. Palette changes must also configure the component theme after Theme::change in main.rs. The Electron logo is the brace-and-cursor geometry in components/Brand.tsx, not a raster asset.
