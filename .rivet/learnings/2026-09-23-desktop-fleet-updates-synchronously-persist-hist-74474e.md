---
title: Desktop fleet updates synchronously persist history before coalescing
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts
  - apps/desktop/scripts/bench-agent-load.mjs
promoted: false
---

# Desktop fleet updates synchronously persist history before coalescing

## Observation
At f96d0010, ClaudeSessionStore.pushUpdate calls dispatchHistoryStore.observe before its 16ms coalescing gate. observe enters transaction before checking hub or tracked membership. Each transaction locks, reloads, clones, reindexes, and JSON-compares the entire history; tracked updates change observedAt/wallMs and persist via atomicWriteFileSync with fsyncSync. Synthetic npm run bench:agent-load on Linux Node 22: at 200 tasks, 120 tracked updates performed 120 reads/writes/fsyncs/locks in 472ms; 120 untracked updates performed 120 reads/locks in 306ms. Main-thread config-lock contention can additionally block up to 250ms per attempt. These are synthetic timings, not a profile of the reporting desktop. Desktop flushSession sends a full snapshot through IPC whereas hubTelemetry compacts before publication; renderer useSessionSnapshots compacts only after receipt and replaces both global maps every update. A synthetic 2000-turn snapshot was 4.07MB JSON versus 24.6KB compact. The benchmark uses production history/compaction code, disposable data, and Node structuredClone as a serialization proxy.

## Impact
Many active sessions and larger persisted history can congest the same Electron main thread that handles spawn requests, while full transcript traffic and fleet-wide React work add renderer load.

## Recommendation
Move high-frequency metric observation off the synchronous persistence hot path while preserving cross-process locking, lifecycle transitions, freshness and workflow decisions. Split compact global updates from active-pane transcript delivery and batch renderer fleet projections. Profile spawn stages separately: worktree/setup before IPC, facade readiness, launch preparation, daemon admission, and provider readiness. Codex currently starts one app-server per managed session; its timeouts are ceilings, not fixed delays.
