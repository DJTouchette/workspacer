---
title: Auto-grow input budgets must change text element rows rather than max height
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - vendor/gpui-component/src/input/state.rs
  - vendor/gpui-component/src/input/mode.rs
  - apps/native/src/ui.rs
promoted: false
---

# Auto-grow input budgets must change text element rows rather than max height

## Observation
Native200% real pixels exposed draft text painting over action controls although outer composer bounds passed. Input max_h does not override TextElement auto-grow min_size.height. Change InputState row budget in place; preserve text/cursor/undo and let internal caret scrolling own the viewport. AutoGrow(max_rows1) must stay multiline: treating it as singleline panics in shape_line on an existing newline document. No prior auto_grow(_,1) production caller was found.
