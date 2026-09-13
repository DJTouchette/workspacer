---
title: Intent background capture belongs before native snapshot coalescing and headless dispatch filtering
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/headless/desktopHost.ts
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
promoted: false
---

# Intent background capture belongs before native snapshot coalescing and headless dispatch filtering

## Observation
Native claudeSessionStore.pushUpdate runs before window checks and 16ms coalescing; evictNow also runs on explicit close. Headless internal.observe filters dispatchHistoryStore-owned attempts, so intent associations must be captured before that filter. Stop can precede the final conversation update, so idle/stopped report changes must still be captured after the state transition.
