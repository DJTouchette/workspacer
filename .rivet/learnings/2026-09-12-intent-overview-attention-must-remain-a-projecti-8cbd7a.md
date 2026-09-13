---
title: Intent overview attention must remain a projection of qualified live sessions
date: 2026-09-12
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/main/shared/intentSummary.ts
  - apps/desktop/src/renderer/src/components/IntentOverview.tsx
promoted: false
---

# Intent overview attention must remain a projection of qualified live sessions

## Observation
Intent work links can contain multiple executions for the same hub-qualified session, and offline federation rows retain pending-slot tombstones. Overview/sidebar attention must deduplicate [hub,sessionId] and exclude offline/stopped/ended rows, then navigate to the original agent response surface without resolving or dismissing approvals itself. Retained intentObservation.summary can be a question/approval blurb as well as assistant prose, so the overview calls it a captured session excerpt, never independently verified evidence.
