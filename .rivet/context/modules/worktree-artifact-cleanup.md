---
title: Worktree artifact cleanup and spawn admission fencing
tags: [worktree, cleanup, artifacts, spawn, maintenance, leases, headless]
related_paths:
  - "services/hub-rs/src/services/worktrees/artifact_cleanup.rs"
  - "services/hub-rs/src/services/worktrees/artifact_cleanup_scheduler.rs"
  - "services/hub-rs/src/services/host_chores.rs"
  - "services/hub-rs/src/backend.rs"
  - "apps/desktop/src/main/services/worktreeArtifactCleanup.ts"
  - "apps/desktop/src/main/services/worktreeArtifactCleanupScheduler.ts"
  - "apps/desktop/src/main/services/worktreeService.ts"
  - "apps/desktop/src/main/headless/stdio.ts"
  - "apps/desktop/scripts/cleanup-agent-artifacts.mjs"
  - "services/claudemon/src/daemon/worktree_admission.rs"
  - "services/claudemon/src/daemon/spawn.rs"
owner: Damien Touchette
last_reviewed: 2026-11-14
---

# Worktree artifact cleanup and spawn admission

## Scope and scheduling

The cleanup core reclaims recognized ignored build/dependency artifacts in
eligible managed linked worktrees. It does not remove worktrees, branches or
source edits. This differs from explicit close-time worktree removal.

There are two implementations of the same rules. The one that runs in the
product is Rust: `services/hub-rs/src/services/worktrees/artifact_cleanup.rs`
(core) and `artifact_cleanup_scheduler.rs`, started by `HostChores`
(`services/hub-rs/src/services/host_chores.rs`) from `Backend::initialize`.
The original TypeScript core, scheduler and CLI remain in `apps/desktop` for
the deprecated Electron app and for manual runs. See
[operator procedure](../../../docs/agent-artifact-cleanup.md) for configuration
and CLI flags. `workspacer cleanup-agents` (`services/hub-rs/src/cli/cleanup.rs`)
runs a pass by hand: dry-run by default, `--apply` enables deletion. Neither the
automatic pass nor the command removes anything on Windows, where the hub does
not record the directory identity the rules need.

The Rust scheduler runs in every backend that owns its engine (native
`--local`, `workspacer serve`), and never in a hub attached to someone else's
engine. The first pass is 60 seconds after backend readiness, then every 15
minutes. Without a configuration directory it does not start. Configuration is
read each pass; default enabled/minimum age is true/one hour. A process-wide
in-flight flag prevents overlap. Each pass runs on its own thread because the
filesystem walks block. `Backend::shutdown` stops chores first: the scheduler
stops scheduling, a running pass loads no further state and removes nothing
more, and shutdown waits up to 10 seconds for it to release its maintenance
lease. It cannot interrupt a removal already in the kernel.

The Rust core needs a directory's device and inode to recheck identity before
removal. On non-Unix platforms it bails with a reported reason, so the Rust
cleanup removes nothing on Windows.

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

Use the Rust core/scheduler fixture tests (`artifact_cleanup_tests.rs`, the
scheduler's own tests; `make test-hub`), the TypeScript core/CLI tests in
`apps/desktop`, and daemon `worktree_admission` tests. The 2026-11-14 review
read the Rust sources and their wiring; it did not run them. The concurrent-admission regression covers shared
ownership and exclusion, not actual provider readiness or sustained fleet load.
A successful spawn acknowledgement/`launch_total` timing ends before first
provider output; benchmark those separately. See [agent spawn](../domains/agent-spawn.md).
