---
title: Native sidebar collapse shares filtering and explicit selection
date: 2026-09-30
confidence: high
related_paths:
  - apps/native/src/ui/sidebar.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native sidebar collapse shares filtering and explicit selection

## Observation
Native expanded sidebar and collapsed rail both derive their rows from visible_sessions, preserving archive/search/project scope rules and navigation_selected feedback. The collapse preference is per-window UI state, not a hub config write. Rail search expands and focuses the existing search entity, retaining its query; session clicks use Command::Select with the concrete session ID. Two GPUI interactions cover draft/filter preservation and explicit rail selection.

## Impact
Keep both navigation presentations on the same filtering and command paths; collapse must not create a new composer or search entity.
