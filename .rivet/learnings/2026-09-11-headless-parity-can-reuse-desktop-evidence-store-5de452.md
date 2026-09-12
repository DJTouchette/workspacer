---
title: Headless parity can reuse desktop evidence stores without Electron
date: 2026-09-11
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/managerRequestService.ts
  - apps/desktop/src/main/services/fleetReviewStore.ts
  - apps/desktop/src/main/services/worktreeService.ts
promoted: false
---

# Headless parity can reuse desktop evidence stores without Electron

## Observation
DispatchHistoryStore, ManagerRequestService and FleetReviewStore import Node filesystem/crypto/shared validation and configService, not Electron; their claudeSessionStore references are type-only. WorktreeService is also Node-compatible and already calls FleetReviewStore. Reusing these through a brain-owned Node host preserves ownership/revision/evidence logic; reimplementing them independently in Go would duplicate large correctness contracts. BriefBoardService additionally imports SQLite sessionHistory, so it requires separating its recent-directory input before reuse.
