---
title: Batch history observations as replayable metrics rather than stale documents
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - docs/desktop-agent-performance.md
promoted: false
---

# Batch history observations as replayable metrics rather than stale documents

## Observation
The first desktop agent performance fix adds DispatchHistoryStore.queueObservation with one 250ms fleet-wide timer. It captures detached usage/statusline and boolean decision state, skips federated snapshots, commits known lifecycle transitions and session end immediately, and drains pending observations inside every explicit history transaction before applying that transaction. The batch is replayed after reload under the existing cross-process lock; failed writes retain it for retry. No delayed full-history document is written. Shutdown stops live bridges then flushes history. Synthetic 200-task/120-update steady-state burst fell from 120 durable writes and about 470ms to one durable write and about 5ms, including final commit; this is not a live desktop latency measurement. Spawn timing records cover renderer IPC roundtrip, worktree preparation, main preflight/facade/assets/integration/admission, Claude stream initialization and Codex readiness/subscription. Renderer console timing does not automatically enter the main logfile.

## Impact
This avoids per-token synchronous persistence without regressing cross-process edits or letting delayed running metrics overwrite a later ended/validated mutation. Crash loss is limited to recently queued metrics; critical transitions remain immediate.

## Recommendation
Use npm run bench:agent-load and docs/desktop-agent-performance.md. Keep explicit observe synchronous for lifecycle callers; hot live snapshots should use queueObservation. Profile full IPC snapshots and fleet React projections separately before the next optimization.
