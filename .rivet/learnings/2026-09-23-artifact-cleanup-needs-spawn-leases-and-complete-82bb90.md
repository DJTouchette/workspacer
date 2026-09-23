---
title: Artifact cleanup needs spawn leases and complete cross-worktree dependency references
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/worktreeArtifactCleanup.ts
  - apps/desktop/src/main/services/worktreeMaintenance.ts
  - services/claudemon/src/daemon/worktree_admission.rs
promoted: false
---

# Artifact cleanup needs spawn leases and complete cross-worktree dependency references

## Observation
Automatic artifact cleanup is scoped to managed linked worktrees, not arbitrary checkouts or ignored folders. New allocations record canonical cwd/root and dev+ino in workspacer-allocation.json under the per-worktree Git administration directory; legacy candidates require wks/ branches. Cleaner and claudemon spawn admission acquire the same exclusive .workspacer-maintenance.lock (pid/token); source worktree preparation also holds it while creating dependency links. Apply requires X-Workspacer-Maintenance:1 from the live daemon. Raw sessions are reloaded under the lease, input/unknown/approval/question/responding all block, missing live cwd fails closed, and stopped timestamps plus newest artifact writes enforce cooldown. Shared refs must use git worktree list, including primary/different roots, and scan inside real node_modules/hidden dirs: pruning artifact trees for candidate discovery is unsafe for reference discovery. The full reference scan is bounded and incomplete scans skip deletion. Own internal symlinks do not block candidate removal; external/shared targets and hardlinked byte accounting are preserved. Locks are not stolen by age and stale locks can require manual verified-owner recovery.

## Impact
This reclaims ignored generated dependencies/builds without erasing dirty source or racing a new provider launch. Task history and renderer teardown alone cannot cover all agent-created worktrees.

## Recommendation
Keep real-Git fixtures for dirty/untracked/force-tracked files, primary/locked worktrees, cross-root and nested dependency links, symlink targets, lease contention and unavailable/old daemon. Use npm run cleanup:agents for preview and -- --apply after rebuilding services. Treat byte totals as conservative estimates, not filesystem-free-space measurements.
