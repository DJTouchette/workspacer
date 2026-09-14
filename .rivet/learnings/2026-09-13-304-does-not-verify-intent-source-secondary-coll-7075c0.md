---
title: 304 does not verify Intent source secondary collection coverage
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceSyncStore.ts
  - apps/desktop/src/main/services/intentSourceStore.ts
promoted: false
---

# 304 does not verify Intent source secondary collection coverage

## Observation
IntentSourceSyncStore.refresh normally bypasses ETags for partial snapshots, but record() previously marked any 304 fresh even when persisted prior coverage was partial. A primary-object 304 must retain incomplete coverage until a full read confirms it; both context packets and Sources UI consume persisted status.
