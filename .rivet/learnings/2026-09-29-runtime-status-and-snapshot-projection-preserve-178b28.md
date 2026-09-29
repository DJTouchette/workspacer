---
title: Runtime status and snapshot projection preserve different kinds of uncertainty
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/src/services/host_status.rs
  - services/hub-rs/src/services/snapshots.rs
promoted: false
---

# Runtime status and snapshot projection preserve different kinds of uncertainty

## Observation
Owned runtime status can refuse a bus request just before its final shutdown phase is published, so lifecycle tests must await that phase instead of demanding synchronous watch updates. Snapshot requestedSelection, resolvedContextWindow and provider statusLine are independent evidence: absent/null resolved windows stay absent and contradicted provider windows must survive unchanged. Explicit owning tests passed in host_status and snapshots.

## Recommendation
Keep status tests tied to real lifecycle transitions and preserve sparse owner/provider claims when changing projection.
