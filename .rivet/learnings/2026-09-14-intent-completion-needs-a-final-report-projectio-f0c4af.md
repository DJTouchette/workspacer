---
title: Intent completion needs a final-report projection and proposal-scoped review CAS
date: 2026-09-14
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/intentCompletionStore.ts
  - apps/desktop/src/main/headless/intentObservations.ts
  - services/claudemon/src/session/conversation.rs
promoted: false
---

# Intent completion needs a final-report projection and proposal-scoped review CAS

## Observation
The former headless summary_source projection caps each assistant event at 800 characters, dropping trailing intent-report contracts. The new completion_source projection returns only the newest assistant reply, never across a newer user message, bounded to 4000 UTF-16 units. Intent idle is not completion: correlate the report to the current run/revision and require owner-host idle with drained descendants, no pending input/tools, and no interruption. Evidence review insertion, proposal validation, run transition and direction allocation share the evidence write transaction; transport receipts remain separate and never prove consumption.

## Recommendation
Keep native/headless final report bounds aligned, preserve explicit malformed/oversized states, and never replay an unknown delivery. Test sparse observation preservation and stale proposal/revision CAS when touching lifecycle or review.
