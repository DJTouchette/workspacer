---
title: Intent Overview combines revision-specific review with existing live session attention
date: 2026-09-13
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/shared/intentSummary.ts
promoted: false
---

# Intent Overview combines revision-specific review with existing live session attention

## Observation
summarizeIntent filters evidence and review to the current workspace revision, while intentSessionAttention projects existing pending approval/question slots from linked live sessions keyed by hub plus sessionId. It excludes offline and ended/stopped sessions. The Work overview therefore combines durable feature review with live agent attention without introducing a separate approval lifecycle.
