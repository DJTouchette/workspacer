---
title: Documentation completion must retain observed implementation limits
date: 2026-09-26
confidence: high
suggested_doc: remote-mobile
promoted: false
---

# Documentation completion must retain observed implementation limits

## Observation
The mobile browser fetcher still uses the returned sequence as its next anchor, unlike the full renderer coalescing-safe item anchor. Web peer seeding skips sparse rows although its event fold accepts them. Attention suppression pruning splits IDs at the first colon, which does not preserve paired IDs. These are current implementation limits, not reasons to claim parity. Source review also found live headless rich handoff and scheduler-driven artifact cleanup absent from old guides.

## Impact
Passing rich-fixture tests cannot justify stronger cross-client guarantees than executable paths provide.

## Recommendation
Document the limitations explicitly and retain source-specific evidence. Keep application fixes as separately scoped work.
