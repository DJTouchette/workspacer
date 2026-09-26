---
title: Worktree artifact cleanup and spawn admission fencing
tags: [worktree, cleanup, artifacts, spawn, maintenance, leases, headless]
related_paths:
  - "apps/desktop/src/main/services/worktreeArtifactCleanup.ts"
  - "apps/desktop/src/main/services/worktreeArtifactCleanupScheduler.ts"
  - "apps/desktop/src/main/services/worktreeService.ts"
  - "apps/desktop/src/main/headless/stdio.ts"
  - "apps/desktop/scripts/cleanup-agent-artifacts.mjs"
  - "services/claudemon/src/daemon/worktree_admission.rs"
  - "services/claudemon/src/daemon/spawn.rs"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Worktree artifact cleanup and spawn admission

## Scope and scheduling

The shared TypeScript cleanup core reclaims recognized ignored build/dependency
artifacts in eligible managed linked worktrees. It does not remove worktrees,
branches or source edits. This differs from explicit close-time worktree removal.
See [operator procedure](../../../docs/agent-artifact-cleanup.md) for configuration
and CLI flags; `npm run cleanup:agents` from `apps/desktop` defaults to dry-run,
with `-- --apply` explicitly enabling deletion. This audit runs fixture tests,
not cleanup against the user's real worktree root.

The scheduler starts 60 seconds after local desktop initialization or the
headless companion's first valid request carrying `daemonURL`, then checks every
15 minutes. Remote-client desktop skips it. Imports alone start nothing.
Configuration is read each pass; default enabled/minimum age is true/one hour.
A module-level in-flight guard prevents overlap within that process. Shutdown
clears timers and refuses later state loads; it does not synchronously cancel
an already executing filesystem operation.

## Eligibility and deletion boundary

Candidates must be linked worktrees under the configured root. Modern allocation
records bind cwd/root and directory identity; older records require a `wks/`
branch. The primary checkout, symlinked/changed identities and Git-locked trees
are excluded. Directory names require matching project metadata, Git must report
no tracked files inside and the directory must be ignored. Nested repositories
and links into other checkouts are preserved.

Age checks cover allocation, stopped-session cooldown and newest artifact write.
Idle is still live and protects the worktree. Unknown/unavailable daemon state
is not an empty fleet. Applying requires the daemon maintenance-support header;
dry-run can inspect without that support but still needs usable session state.

Dependency references are rescanned under the maintenance lease across registered
worktrees, including hidden/dependency directories and other roots. Symlinks are
inspected without traversing targets. Incomplete/over-budget scans skip cleanup.
Before removal the core rechecks directory identity and canonical path. Byte
estimates exclude symlink targets and multiply-linked files; reported bytes are
not a measured change in free disk space. Inspect skipped reasons and errors;
a successful invocation does not imply anything was deleted.

## Cross-process maintenance and launch fencing

Cleanup and worktree creation use the same per-Git-directory maintenance file
as daemon admission. Both managed and PTY daemon spawn acquire
`WorktreeAdmission` before registering the session. Nested cwd aliases resolve
to the canonical Git administration directory. Primary checkouts/ordinary
directories do not acquire this linked-worktree fence.

Within one daemon, overlapping launches reference-count an exact owned fence;
they no longer refuse each other merely because a sibling launch holds it.
Acquisition and final release share a mutex. Ownership requires the saved PID
**and random token** to match the registry's owner; PID alone cannot adopt a
foreign/stale file. Other processes and maintenance still require exclusive
ownership. The final guard releases only its still-owned file.

Malformed, replaced or poisoned ownership fails closed. Locks do not expire by
age. A crash can require manual investigation; do not advise deleting a lock
solely because it is old. Confirm recorded owners are stopped before removing
an abandoned `.workspacer-maintenance.lock` from the relevant Git directory.

## Validation

Use cleanup core/scheduler fixture tests, CLI argument/config tests and daemon
`worktree_admission` tests. The concurrent-admission regression covers shared
ownership and exclusion, not actual provider readiness or sustained fleet load.
A successful spawn acknowledgement/`launch_total` timing ends before first
provider output; benchmark those separately. See [agent spawn](../domains/agent-spawn.md).
