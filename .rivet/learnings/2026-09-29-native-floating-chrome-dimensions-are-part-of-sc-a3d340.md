---
title: Native floating chrome dimensions are part of scroll restoration
date: 2026-09-29
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/scroll.rs
  - apps/native/src/ui/tools.rs
promoted: false
---

# Native floating chrome dimensions are part of scroll restoration

## Observation
The native chat header and composer are measured by deferred canvas callbacks in ui.rs. Transcript top/tail padding and Jump to latest position use those measurements; changing header height must preserve the callback's adjustment to scroll_anchor.offset_in_item while follow is paused. A flatter header and collapsed default tool cards can change geometry substantially while existing reading/expansion tests protect anchoring.
