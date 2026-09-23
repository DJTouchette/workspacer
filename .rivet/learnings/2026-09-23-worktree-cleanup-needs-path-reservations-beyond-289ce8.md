---
title: Worktree cleanup needs path reservations beyond cached fleet snapshots
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/headless/desktopHost.ts
  - apps/desktop/src/main/headless/stdio.ts
  - services/hub/cmd/brain/desktophost.go
  - apps/desktop/src/main/services/worktreeService.ts
  - services/hub/internal/quiescence/quiescence.go
promoted: false
---

# Worktree cleanup needs path reservations beyond cached fleet snapshots

## Observation
Headless Go dispatch delegates worktree allocation to TypeScript: brain/desktophost.go internal.prepareSpawn runs desktopHost.ts createWorktree, then Go spawnCore registers the daemon session, followed by internal.acceptSpawn/cancelSpawn. The allocation-to-registration interval has no daemon row, and stdio.ts runs calls concurrently. The existing desktop.worktreeRemove check reads request-captured snapshots and matches only (liveCwd || cwd) === target; it misses nested/shared paths and pending admissions. Direct IPC worktree removal calls removeAgentWorktree without a liveness check. fleet.quiescence is not a no-live-session gate: mode=input and ambientState=idle count as resting. workspacer CLI currently has no storage/cleanup command.

## Impact
A cleanup sweep based only on ended parent rows, absent cached snapshots, or fleet quiescence can delete artifacts in an idle but live session, child-shared worktree, or newly allocated launch before its daemon registration.

## Recommendation
Use shared canonical path reservations covering allocation/setup through daemon acknowledgement; serialize cleanup with reservations across relevant processes. Query the local daemon fresh with state_only/include_archived/include_empty and fail closed on unreadable or unavailable state. Treat all non-stopped sessions as blockers and compare both original and live cwd using path overlap. Discover old worktrees from configured roots plus verified Git linked-worktree metadata rather than bounded dispatch history. Reuse shared TS cleaner in desktopHost and expose an explicit named CLI route.
