---
title: History rows cannot serve manager dispatch history
date: 2026-09-06
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/recentSessions.ts
  - apps/desktop/src/main/services/sessionStore/analyticsWriter.ts
  - apps/desktop/src/renderer/src/lib/sessionHistoryGroups.ts
promoted: false
---

# History rows cannot serve manager dispatch history

## Observation
The Sessions/History pane merges per-project Claude transcripts with daemon resumable rows; session_history analytics is only names/cost. Neither source by itself records a manager-owned dispatch/task edge or durable workflow membership.

## Impact
A Recent agents/whole-loop pane must not infer manager tasks from History ordering, labels, parent IDs, or transcript prose.

## Recommendation
Add a bounded lifecycle-owned dispatch/workflow record only if no existing correlation carries the needed task and step identities.
