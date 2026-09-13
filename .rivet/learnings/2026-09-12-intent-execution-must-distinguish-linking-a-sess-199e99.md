---
title: Intent execution must distinguish linking a session from delivering pinned context
date: 2026-09-12
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - apps/desktop/src/main/shared/intentWorkspace.ts
promoted: false
---

# Intent execution must distinguish linking a session from delivering pinned context

## Observation
useAgentManager.spawnAgent already sends kickoffMessage as the spawn's first message and invokes onSessionReady after recording the agent but before selecting it. A throwing onSessionReady can make a successful spawn appear failed. Intent execution should persist its launch attempt before spawning, pin and send the context packet through kickoffMessage, and treat post-spawn link-write failures as reconciliation rather than retryable spawn failures. Attaching an existing session must not imply that context was delivered.
