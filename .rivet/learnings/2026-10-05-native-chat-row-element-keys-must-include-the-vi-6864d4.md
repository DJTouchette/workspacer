---
title: Native chat row element keys must include the viewed child
date: 2026-10-05
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui/transcript.rs
  - apps/native/src/ui/work.rs
  - apps/native/src/ui.rs
promoted: false
---

# Native chat row element keys must include the viewed child

## Observation
Every native Transcript numbers row.key from 0. Row TextView/tool-card keys were live:{selected}:{row.key}, and selected stays the parent while a child is viewed, so parent<->child and sibling switches reused the previous transcript's TextView keyed state: gpui-component then paints the OLD parsed text and only reparses after its 200ms debounce. Also update_view fed the parent's TurnClock the child's transcript and painted parent turn durations (duration_labels keyed by row.key) on child rows.

## Impact
Stale content flash on every child switch; wrong 'Took Ns' labels on child replies.

## Recommendation
Use Workspace::chat_owner() ({session}/{child agent}) for any element key derived from row.key; keep view.selected for request ownership. Turn clocks and duration labels only apply when view.child is None. Regression tests: ui/tests/switching.rs.
