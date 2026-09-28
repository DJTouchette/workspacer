---
title: Native keyboard page-up can clamp back to the tail
date: 2026-09-27
confidence: medium
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/navigation.rs
promoted: false
---

# Native keyboard page-up can clamp back to the tail

## Observation
During native UI visual checks, repeated Ctrl+U showed Jump to latest but failed to move far into history. navigation.rs page() calls ListState::scroll_by(-half viewport). GPUI 0.2.2 logical_scroll_top() represents bottom-follow as item_count with offset 0; scroll_by starts from that end-of-items coordinate, so a half-viewport subtraction can still land past the maximum top offset and clamp back to bottom. Mouse wheel uses a different normalized scroll path. This predates floating header work; page() needs normalization from the actual visible top before applying its delta.
