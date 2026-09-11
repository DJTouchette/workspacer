---
title: Task tracking opt-out must preserve parent ownership for paired dispatch and wakes
date: 2026-09-11
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/services/pairedDispatch.ts
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Task tracking opt-out must preserve parent ownership for paired dispatch and wakes

## Observation
Task history is separate from session parentage and paired transport admission. Clearing dispatchOwnerSessionId to avoid a Task breaks paired dispatch's authenticated owner check. The new trackTask:false travels through the ordinary admission path, suppresses only DispatchHistoryStore validation/acceptance, and retains parent/wake routing plus the remote dispatch journal. Conflicting task/workflow/retry links are rejected rather than silently detached.
