---
title: Intent status-only revisions leave accepted review historical
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/shared/intentSummary.ts
  - apps/desktop/src/main/shared/intentWorkspace.ts
promoted: false
---

# Intent status-only revisions leave accepted review historical

## Observation
IntentWorkspaceStore.update increments revision on every save, including status-only changes. Criteria and summarizeIntent reviews are revision-scoped: accepting work then saving Complete leaves acceptance on the prior revision. Only draft/active/review/complete exist. PR links have no lifecycle state or synchronization.

## Recommendation
Document the manual PR/rework/merge workflow and status-only revision effect; keep workspace status, evidence review, session observations and external PR state distinct.
