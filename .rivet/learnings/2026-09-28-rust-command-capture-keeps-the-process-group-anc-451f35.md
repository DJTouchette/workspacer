---
title: Rust command capture keeps the process-group anchor unreaped through cleanup
date: 2026-09-28
suggested_doc: hub-process-supervision
related_paths:
  - Use owned_process capture/capture_limits/capture_input instead of raw Command.output or a killpg Drop after Child.wait. Preserve source command output limits and attach deliberate deadlines.
promoted: false
---

# Rust command capture keeps the process-group anchor unreaped through cleanup

## Observation
The prior worktree/Git capture helper waited and reaped the leader before Drop sent killpg, allowing a stale numeric PID group signal. Shared services/owned_process.rs now observes Unix leader exit using waitid WNOWAIT, kills only its verified fresh separate group while the original child remains unreaped, and then reaps. Windows uses a retained per-child kill-on-close Job handle. The helper bounds stdin/stdout/stderr and deadlines, scrubs host tokens, and refuses numeric signals after foreign reaping. It does not claim ownership of Unix descendants that escape that group.
