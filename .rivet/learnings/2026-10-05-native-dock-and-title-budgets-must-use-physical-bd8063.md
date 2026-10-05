---
title: Native dock and title budgets must use physical windows and actual rem gutters
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/chrome.rs
promoted: false
---

# Native dock and title budgets must use physical windows and actual rem gutters

## Observation
A scaled px(720) test window grows along with zoom and misses real minimum-window overflow. Hold viewport at gpui::px(720)xgpui::px(480), apply Typography, and test100/125/150/200%. GPUI root rem follows the theme font size: title pt_3+pb_5 costs two window.rem_size(), not fixed32px. Long notices and unbounded attachments can overlap a non-scrolling composer despite the phantom-scroll fix. Bound ancillary scroll areas, reserve composer geometry, and permit zero transcript gap at extreme density; do not hide Working status.
