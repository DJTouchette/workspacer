---
title: Progress decision flags and orphan guidance are part of the worker contract
date: 2026-09-30
confidence: high
related_paths:
  - services/hub-rs/src/services/progress.rs
  - services/hub-rs/src/services/agent_ops.rs
promoted: false
---

# Progress decision flags and orphan guidance are part of the worker contract

## Observation
The headless-port audit reproduced a real bus report accepting needsDecision as a string, delivering a non-decision note and consuming its budget. Validate boolean-or-null before delivery/budget mutation. The orphan projection retained candidates but had lost the empty-fleet message and explicit do-not-guess adoption guidance; restore those without changing candidate ownership. Exact before/final logs are /tmp/workspacer-headless-port-before.log and /tmp/workspacer-headless-port-final.log (headless3 and progress4 passed).

## Recommendation
Preserve typed wire semantics and actionable orphan guidance alongside ownership/liveness checks; keep malformed-input negative controls with valid same-note retry.
