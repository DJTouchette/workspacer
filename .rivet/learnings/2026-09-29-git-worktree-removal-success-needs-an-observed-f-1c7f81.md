---
title: Git worktree removal success needs an observed filesystem postcondition
date: 2026-09-29
promoted: false
---

# Git worktree removal success needs an observed filesystem postcondition

## Observation
Actual Windows containment CI for e3f06ac4 passed native PTY stopped-state recovery but then observed Worktrees.remove return ok while its selected checkout directory still existed. Git success alone is therefore not sufficient evidence for this API. The service now checks symlink_metadata under the maintenance lease, permits a bounded two-second Windows delete-pending settling window, and reports explicit failure if the path remains or cannot be inspected. It does not force Git or recursively remove leftovers/recreated paths. The original real integration disappearance assertion is unchanged, with bounded remaining-entry diagnostics. The exact source of the Windows residual path (delete-pending handles versus recreation) is not yet established; do not attribute it to child jobs without evidence.
