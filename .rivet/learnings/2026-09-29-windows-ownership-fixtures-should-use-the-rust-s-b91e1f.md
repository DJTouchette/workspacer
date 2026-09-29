---
title: Windows ownership fixtures should use the Rust self-test child instead of PowerShell
date: 2026-09-29
promoted: false
---

# Windows ownership fixtures should use the Rust self-test child instead of PowerShell

## Observation
Actual61034c67 Windows containment passed junction guards and real worktree removal, then two PowerShell-based ownership fixtures timed out awaiting descendant readiness before any termination assertion. Existing Rust self-test std/Tokio immediate-fork fixtures passed. The after-assignment and ConPTY fixtures now use that same current_exe helper: command-local marker reports an actual cmd child PID; only the after-assignment case gates on stdin. ConPTY still forks without input. Tests still retain the original descendant process handle and require exit within five seconds after owned cleanup; production code and assertion deadlines are unchanged. Exact-source Windows compile passes; execution of fixture update awaits CI.
