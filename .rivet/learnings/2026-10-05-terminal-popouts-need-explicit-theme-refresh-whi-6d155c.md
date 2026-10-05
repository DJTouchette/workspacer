---
title: Terminal popouts need explicit theme refresh while the panel is hidden
date: 2026-10-05
confidence: high
suggested_doc: theme-system
related_paths:
  - apps/native/src/ui/terminal.rs
  - apps/native/src/ui/typography.rs
promoted: false
---

# Terminal popouts need explicit theme refresh while the panel is hidden

## Observation
TerminalPalette was updated only when opening/rendering the main terminal panel. Pop out sets terminal.open=false, so changing themes on Settings left both the popout chrome and ANSI cells on the previous palette. apply_typography now refreshes the terminal palette, terminal views and popout window alongside the file-viewer popout. A GPUI interaction regression cycles all eight themes after popping out.
