---
title: Project tracker connections must not replace pinned source connection identity
date: 2026-09-14
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceStore.ts
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
promoted: false
---

# Project tracker connections must not replace pinned source connection identity

## Observation
IntentSourceStore imports and refreshes through IntentSourceSyncStore using each source's stored canonical URL and credentialEnv. Account leasing and restart scheduling are derived from those pinned fields. Project integrations therefore belong beside sources in the Intent database; changing registry metadata must only affect new attachments. The shared SOURCE_ACTIONS dispatch in intentWorkspaceRequest is used by native and headless Intent operations.
