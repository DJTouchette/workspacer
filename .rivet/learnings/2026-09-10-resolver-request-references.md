---
title: Request references must be assigned before resolver deletes original content
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerRequestService.ts
  - apps/desktop/src/main/shared/requestReferences.ts
  - apps/desktop/src/renderer/src/lib/notificationStore.ts
promoted: false
---

The request resolver deletes userContent on successful resolution. Reference
mapping must therefore run inside requestTransaction before that deletion, using
original inbox text and the resolved intent mapping. Separate manager reference
tools do not automatically preserve pasted URLs. Transaction rollback retains
content when a mapping or singleton PR conflict needs a manager decision.

Renderer postNotification normally escalates to OS notifications when unfocused.
Routine manager-request receipts explicitly opt out while honoring inAppToasts.
