---
title: Model fences must outlive cached context display occupancy
date: 2026-09-01
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/providers/codex.rs
promoted: false
---

# Model fences must outlive cached context display occupancy

## Observation
The bounded model-confirmation fence correctly prevented stale context health, but UsageAcc still emitted its cached context_used_pct/context_window_size after that fence expired. At a model boundary, fence both display fields as WaitingForRuntimeUsage until a newly received runtime-correlated pair is validated; do not clear cumulative token/cost counters.

## Impact
Without the separate display fence, a post-switch Codex status tick can falsely show predecessor 180000/200000 occupancy under the successor model.

## Recommendation
When adding model/provider boundaries, test more ticks than the bounded confirmation budget and assert context health plus both display occupancy fields remain absent until a fresh runtime pair arrives.
