---
title: Fleet must restore retained frames before removing their destination
date: 2026-09-06
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/components/claude/RetainedSessionChat.tsx
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - apps/desktop/scripts/checkFleetWorkflow.mjs
promoted: false
---

# Fleet must restore retained frames before removing their destination

## Observation
A stable React portal alone preserves React state but does not preserve an iframe document if its Fleet destination is disconnected first. RetainedSessionChat registers existing pane homes and Fleet destination layout cleanup moves containers home while connected, using moveBefore. The isolated Chromium harness verifies both the real composer and an input inside the sandboxed HTML card survive switch and collapse. A session can also outlive its attached chat pane when other workspace panes remain; ensureAgentChat restores only a missing ordinary viewer by session metadata without changing the active agent or spawning a process.
