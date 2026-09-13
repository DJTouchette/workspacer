---
title: Source publishing must preserve unknown receipts and provider revision race
date: 2026-09-12
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentSourceStore.ts
  - apps/desktop/src/main/services/intentSourceAdapters.ts
promoted: false
---

# Source publishing must preserve unknown receipts and provider revision race

## Observation
IntentSourceStore persists an unknown comment attempt before any provider I/O; confirmed read/preflight failure permits explicit retry, but any post dispatch exception stays unknown and cannot replay even after restart. Jira Cloud comment POST and ADO comment POST do not expose an atomic condition on issue/work-item revision, so checking source digest immediately before POST detects observed drift but cannot rule out a racing remote edit. ADO 7.1-preview.4 docs use commentId in the example and id in the schema; accept either receipt field.

## Recommendation
Keep user-visible preflight race disclosure, mock unknown/reopen/claim-write-failure paths, and do not describe source revision pinning as an atomic provider constraint.
