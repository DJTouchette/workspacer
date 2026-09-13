---
title: Promotion preconditions must inspect the pinned destination
date: 2026-09-13
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentKnowledgeStore.ts
  - apps/desktop/src/main/services/intentKnowledgeStore.test.ts
  - apps/desktop/src/renderer/src/components/IntentKnowledge.tsx
  - apps/desktop/src/renderer/src/components/IntentEvidence.tsx
promoted: false
---

# Promotion preconditions must inspect the pinned destination

## Observation
When a knowledge promotion retains a directory FD for its final rename, rechecking only the original pathname is insufficient: that pathname can be replaced by a new ordinary directory containing the old baseline while the pinned original file is edited. intentKnowledgeStore now checks both the bytes through the pinned anchoredTarget and the ordinary-path baseline before replacement; its parent-swap regression preserves both concurrent copies. Knowledge 'written' receipts are historical verification; the renderer must not describe them as a current file match on subsequent reads.

## Recommendation
Keep the anchored read-before-rename regression and historical receipt wording. When a read-only diff preview runs during Evidence revision refresh, do not invalidate the independent evidence-list generation; otherwise current criteria remain stale.
