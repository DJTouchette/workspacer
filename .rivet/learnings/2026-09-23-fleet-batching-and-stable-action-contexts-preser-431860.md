---
title: Fleet batching and stable action contexts preserve urgent decisions
date: 2026-09-23
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts
  - apps/desktop/src/renderer/src/contexts/AttentionContext.tsx
promoted: false
---

# Fleet batching and stable action contexts preserve urgent decisions

## Observation
Routine renderer fleet snapshot promotion now shares a 100ms timer, coalesces per session, and preserves status-map identity for unchanged ambient states. New sessions, status/ambient transitions, hub-offline changes and approval/question payload changes publish immediately; prune/end removes queued observations. A per-request changed-session set protects hydration from newer push/termination events without accumulating lifetime tombstones. Attention actions now read the latest snapshots through refs and have a separate provider; FleetMessage chips subscribe only to actions and an indexed agent directory. AgentCard decision lookup uses one per-feed index rather than scanning the feed per card. Regression tests cover batching, lifecycle/hydration races, context render isolation and current transport routing.

## Impact
Reduces fleet-wide publications and prevents hidden transcript chips from following unrelated token streams while preserving prompt decisions and termination semantics.

## Recommendation
Keep urgent paths immediate and retain latest-state refs if adding routing actions; do not recombine stable actions with volatile snapshots. The full fleet feed still recomputes on each batched publication.
