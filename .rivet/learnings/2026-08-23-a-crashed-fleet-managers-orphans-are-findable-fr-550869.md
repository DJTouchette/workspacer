---
title: A crashed Fleet Manager's orphans are findable from parentSessionId, but the dead predecessor itself is unidentifiable
date: 2026-08-23
author: session: parentSessionId-on-agents.list
confidence: high
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - services/hub/cmd/mcp/help.go
  - apps/desktop/src/renderer/src/lib/fleetManager.ts
promoted: true
---

# A crashed Fleet Manager's orphans are findable from parentSessionId, but the dead predecessor itself is unidentifiable

## Observation
Manager succession has two paths. With a handoff file, the successor reads both ids off it and calls adopt_workers (agents.reparent → claudeSessionStore.reparentChildren). Without one (the manager CRASHED), the only trace left is the workers themselves: their `parentSessionId` still points at the dead session. Since 2026-08-23 the desktop's `agents.list` row carries that field, so the successor can group rows by parent and treat any parent id that has no row of its own as a dead parent with live children — the `fromSessionId` adopt_workers needs. What it CANNOT do is decide which dangling parent was ITS predecessor: claudeSessionStore evicts an ended session ~30s after SessionEnd, so the dead manager has no row at all — no label, no isSupervisor, no cwd — and nothing distinguishes "a dead manager" from "a dead worker that had subagents" or one dead manager from another. With more than one dangling parent it is still a judgement call (compare the live children's cwd/label against what the successor was told to take over). The case is narrowed, not closed.</observation>
<parameter name="impact">Anyone writing successor automation on top of this field will assume a single dangling parent is always the predecessor. It usually is, and adopt_workers is refused for an unreachable destination — but adopting the WRONG group silently re-routes another manager's wakes.

## Recommendation
Closing the remaining gap needs a durable record of the dead manager (a tombstone row, or isSupervisor/label surviving eviction), not more fields on the live row. Until then, treat multiple dangling parents as ambiguous. Also note adopt_workers is deliberately NOT federated (cmd/mcp/main.go): a worker and its manager live on the same hub, so a remote row's parentSessionId names a session on the PEER and is only adoptable there.

## Disposition
Merged with 039970 (dead-manager identity) and 3c6d9d (agents.list shape) into one consolidated section in .rivet/context/domains/session-lifecycle.md (manager succession is memory-only, 2026-08-23).
