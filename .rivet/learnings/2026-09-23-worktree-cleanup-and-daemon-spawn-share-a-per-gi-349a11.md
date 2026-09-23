---
title: Worktree cleanup and daemon spawn share a per-gitdir admission fence
date: 2026-09-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/claudemon/src/daemon/worktree_admission.rs
  - services/claudemon/src/daemon/spawn.rs
  - services/claudemon/src/daemon/api.rs
promoted: false
---

# Worktree cleanup and daemon spawn share a per-gitdir admission fence

## Observation
Both daemon spawn endpoints now acquire a create_new .workspacer-maintenance.lock in the canonical gitdir named by the nearest ancestor .git FILE, retaining the guard through engine start/session registration. JSON ownership is {pid,token}; drop rereads and removes only matching ownership. Primary .git directories stop discovery without locking. GET /health still returns plaintext ok and now advertises X-Workspacer-Maintenance: 1 so cleaners can refuse apply against old adopted daemon binaries. Rust does not steal locks based on age or an unverified PID because no reusable cross-platform liveness helper exists.

## Impact
When the TS cleaner holds the same lock while refreshing live daemon state and deleting generated artifacts, a competing spawn either publishes its row first or receives HTTP409 with retry guidance; it cannot start in the check-delete gap. Older daemons remain detectable and must not be treated as guarded.

## Recommendation
Keep the lock filename, JSON ownership, health header, and canonical gitdir resolution identical in Rust and TypeScript. Do not treat quiescence as no-live-session proof. Recover stale lock files only after positively proving owner death and rechecking identity, never solely from mtime.
