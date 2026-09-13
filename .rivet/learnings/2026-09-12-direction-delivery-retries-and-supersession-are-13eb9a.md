---
title: Direction delivery retries and supersession are independent durable state transitions
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/intentSteeringStore.ts
  - apps/desktop/src/main/shared/intentWorkspace.ts
promoted: false
---

# Direction delivery retries and supersession are independent durable state transitions

## Observation
Intent steering records pin saved intent revision, execution, qualified target, and exact message before dispatch. An unknown attempt is committed before I/O; accepted and unknown attempts block all automatic or differently-keyed retries. Only an explicit new attempt after confirmed failure can send again. Supersession changes desired direction history but never rewrites prior receipts or retracts a queued message. Final receipt writes reread the direction inside the transaction so replacements saved during I/O survive.
