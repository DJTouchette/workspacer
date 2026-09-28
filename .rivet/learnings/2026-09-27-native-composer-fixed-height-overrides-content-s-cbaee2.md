---
title: Native composer fixed height overrides content-sized input
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
promoted: false
---

# Native composer fixed height overrides content-sized input

## Observation
The native conversation composer used both InputState rows(3) and an explicit Input height of 88px (48px below 620px window height), plus separate hint/footer rows. GPUI Component 0.5.1 supports InputState auto_grow(min_rows, max_rows); omit the Input height override to let it shrink for short drafts and grow for multiline content. The header's CONVERSATION overline and 16px vertical padding also consumed transcript space.
