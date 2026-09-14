---
title: Manager succession currently transfers routing parentage only
date: 2026-09-08
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/supervisorNudge.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/enrich.go
promoted: false
---

# Manager succession currently transfers routing parentage only

## Observation
The established reparentChildren/adopt_workers path changes live and pending workers' parentSessionId and redirects coalesced finish wakes; it records no manager lineage or task/workflow ownership transfer. A seamless Fleet Manager replacement must add host-owned transactional state beyond this method.

## Impact
Avoids treating worker adoption as complete managerial handoff and losing task attribution or duplicate/missed completion work.

## Recommendation
Design replacement around host-controlled prepare/validate/spawn/reparent/commit with durable operation state and explicit task/workflow ownership semantics.
