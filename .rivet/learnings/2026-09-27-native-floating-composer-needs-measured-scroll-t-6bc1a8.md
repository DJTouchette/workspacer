---
title: Native floating composer needs measured scroll-tail padding
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
promoted: false
---

# Native floating composer needs measured scroll-tail padding

## Observation
The native conversation dock overlays the full-height GPUI List. Its actual laid-out bounds are measured with an absolute top:0/left:0 canvas and fed back as List bottom padding so the final message clears the growing composer and approval panels, while older rows still paint behind the dock. GPUI ListState::bounds_for_item returns None for bottom-follow mode because logical_scroll_top reports item_count; geometry tests must inspect painted row bounds with debug_selector instead.
