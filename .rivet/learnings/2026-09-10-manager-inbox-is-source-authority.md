---
title: Manager inbox authority is independent of provider acknowledgement
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerRequestService.ts
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - apps/desktop/src/main/services/managerReplacementService.ts
promoted: false
---

The manager explicitly approved retaining bounded original content in a local
authenticated inbox while unresolved. This supersedes the metadata-only blocker
in the earlier request-identity learning. Unknown provider acknowledgement is
eligible for inbox interpretation but never automatic provider replay. Store
logical request IDs separately from attempt IDs; preserve them in handoff holds,
in-flight records and ownership transfer. Delete original content on resolution.
