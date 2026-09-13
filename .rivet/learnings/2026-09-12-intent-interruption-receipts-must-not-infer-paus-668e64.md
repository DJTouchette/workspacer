---
title: Intent interruption receipts must not infer paused state
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - apps/desktop/src/main/services/intentControlDelivery.ts
promoted: false
---

# Intent interruption receipts must not infer paused state

## Observation
claudemonSessionClient.signal resolves on HTTP success and returns no lifecycle receipt; SIGINT can leave background subagents running and PTY interrupted turns emit no Stop hook. Intent control must retain transport acceptance separately from observed session state, and treat signal exceptions as uncertain unless refusal was proven before dispatch. Both local and peer legacy claude.signal lack a built-in manager fence, so intent adapter must fence local delivery explicitly.
