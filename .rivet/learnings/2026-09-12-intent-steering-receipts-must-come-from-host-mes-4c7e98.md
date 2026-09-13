---
title: Intent steering receipts must come from host messaging, not renderer-reported success
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - services/hub/cmd/brain/managerrequests.go
promoted: false
---

# Intent steering receipts must come from host messaging, not renderer-reported success

## Observation
claudeMessage's native peer branch collapses failures to ok:false, while local claudemonSessionClient.message distinguishes ManagerDeliveryRejected from ambiguous network errors and may return handoff-queued. Headless claudemonClient.deliverCaptured already classifies explicit HTTP rejections separately from unknown outcomes. Intent steering should use host adapters around these existing messaging primitives, pin hub+session identity from the linked execution, record unknown before I/O, and never accept caller-supplied receipt statuses.
