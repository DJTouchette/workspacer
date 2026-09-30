---
title: Native compact layout retains sidebar and large conversation dock
date: 2026-09-30
confidence: high
related_paths:
  - apps/native/src/ui.rs
  - apps/native/docs/ui-polish-compact.png
promoted: false
---

# Native compact layout retains sidebar and large conversation dock

## Observation
At current HEAD, Workspace::render uses a 232px sidebar below 900px width and a conversation dock capped at 55 percent of the full window height. The saved 720x480 ui-polish-compact.png capture shows approval plus composer leaving very little transcript visible. This is a source and saved-capture review, not a fresh runtime check.

## Impact
Compact-window polish should recover conversation space while preserving measured header/dock scroll geometry.
