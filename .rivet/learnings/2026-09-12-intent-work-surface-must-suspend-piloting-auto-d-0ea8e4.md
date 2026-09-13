---
title: Intent work surface must suspend piloting auto-dismiss without changing agent pane identity
date: 2026-09-12
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/contexts/AttentionContext.tsx
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
promoted: false
---

# Intent work surface must suspend piloting auto-dismiss without changing agent pane identity

## Observation
AttentionContext auto-dismisses all items for the active agent while viewLevel is piloting. A new Work surface covering that agent must pass fleet attention semantics even if ui.mode is focus, otherwise approvals/questions for the hidden pane disappear from the feed. Keep the sidebar's existing altitude separate so merely opening Work does not auto-collapse it. The new shared intent service uses node:sqlite and its own versioned file through native IPC and desktop-host owner RPC, avoiding the existing different native/headless analytics database paths.
