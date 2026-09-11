---
title: Direct implementation starter omits review; implementer-led discovery needs a distinct default
date: 2026-09-11
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/shared/fleetWorkflow.ts
  - apps/desktop/src/main/shared/fleetWorkflowSelection.ts
promoted: false
---

# Direct implementation starter omits review; implementer-led discovery needs a distinct default

## Observation
The existing direct-implementation starter contains only implementation and intentionally omits independent review. Changing the default to it would weaken the agreed review policy. A separate implement-review starter keeps required independent review, while the existing scout-implement-review ID and pinned tasks remain unchanged. Explicit global/project selections still override the fallback default.
