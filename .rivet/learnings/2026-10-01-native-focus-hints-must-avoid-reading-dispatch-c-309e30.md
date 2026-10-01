---
title: Native focus hints must avoid reading dispatch contexts during first render
date: 2026-10-01
confidence: high
related_paths:
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# Native focus hints must avoid reading dispatch contexts during first render

## Observation
Window.context_stack assumes a populated GPUI dispatch tree and panics if called from the native sidebar first render, before nodes exist. The sidebar keyboard hint instead checks the workspace and owned input focus handles, which are safe before first paint. Full UI tests and a real window exposed the first-frame panic before publication.

## Impact
Read focus handles during rendering; reserve dispatch-context inspection for input handling after the tree has been built.
