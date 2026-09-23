---
title: Drain queued fleet observations before authoritative refresh
date: 2026-09-23
confidence: high
suggested_doc: renderer-live-state-hooks
related_paths:
  - apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts
promoted: false
---

# Drain queued fleet observations before authoritative refresh

## Observation
A routine snapshot received before a reconnect/list refresh was not in the during-refresh changed-session set. The fetch could publish newer state, then the old 100ms timer reverted it. A failing hook regression reproduced this ordering. useSessionSnapshots now drains pending observations before starting each authoritative request. Successful hydration also removes decision signatures for absent sessions unless a push changed that session during the request, so returning sessions count as new and publish immediately.

## Impact
Prevents stale snapshot rollback after reconnect and avoids lifetime accumulation of vanished-session decision metadata.

## Recommendation
Keep explicit tests for pre-fetch queued updates as well as updates received during a fetch; these are different ordering cases.
