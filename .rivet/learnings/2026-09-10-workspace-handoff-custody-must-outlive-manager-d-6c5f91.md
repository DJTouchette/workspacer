---
title: Workspace handoff custody must outlive manager delivery
date: 2026-09-10
confidence: high
suggested_doc: modules/task-handoff
related_paths:
  - services/hub/cmd/brain/taskhandoff.go
  - apps/desktop/src/main/services/pairedDispatch.ts
promoted: false
---

# Workspace handoff custody must outlive manager delivery

## Observation
The paired dispatch wake gate requires a live local manager and refuses uncertain message delivery. Git and artifact custody must run under a separate peer/task/session gate, or a finished worker cannot return verified bytes while its manager is being replaced. A local predecessor can also have a different committed HEAD in its allocated worktree than the configured project root.

## Impact
Tying transfer to wake readiness can strand reports or execute the next task at the wrong checkpoint.

## Recommendation
Keep the brain as the sole transfer-file owner in catalog and full scope; bind exact commit, manifest and allocation identity to admission, then handle manager wake separately.
