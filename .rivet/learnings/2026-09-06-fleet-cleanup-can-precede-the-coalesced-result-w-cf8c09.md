---
title: Fleet cleanup can precede the coalesced result wake
date: 2026-09-06
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetReviewStore.ts
  - apps/desktop/src/main/services/supervisorNudge.ts
promoted: false
---

# Fleet cleanup can precede the coalesced result wake

## Observation

supervisorNudge coalesces finished results for 1500 ms, while renderer worker removal independently invokes worktreeRemove. Capturing only when composing the wake can therefore lose a clean allocated worktree. The local Fleet review store now captures before removeAgentWorktree executes git worktree remove, and the wake reuses that pre-removal capture only when the worktree has disappeared. Captured bytes and originating-manager ownership survive removal; stopped filesystem grants remain unchanged.
