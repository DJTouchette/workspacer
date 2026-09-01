---
title: Context-health absence needs a tri-state store signal
date: 2026-08-31
author: Codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/session/state.rs
promoted: false
---

# Context-health absence needs a tri-state store signal

## Observation
`UsageAcc` can correctly clear its `context_health` after a newer partial sample, but `SessionStore::reconcile_context_health` historically interpreted every absent value as an unrelated cost/rate tick and retained the prior pair. `Option<ContextHealth>` alone cannot distinguish "no context update" from "context was updated and is now untrustworthy."

## Impact
Partial successor samples clear ContextHealth inside UsageAcc and then the store resurrects the stale pair, making threshold wakes trust evidence the producer explicitly invalidated.

## Recommendation
Carry a non-serialized consumed-once update marker beside `context_health`: false retains across unrelated ticks, true + None clears. Test the full UsageAcc -> apply_updates -> SessionStore path, not only the accumulator in isolation.
