---
title: Manager task reference writes must recheck ownership under the history lock
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowService.ts
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Manager task reference writes must recheck ownership under the history lock

## Observation
ownerTask checks before reference mutation can precede another process adopting the task. updateReferencesByManager now invokes the live owner gate after reloading inside the transaction, before CAS or mutation; conflict responses recheck ownership before exposing references. Manager edits preserve human entries and host-only waivers.
