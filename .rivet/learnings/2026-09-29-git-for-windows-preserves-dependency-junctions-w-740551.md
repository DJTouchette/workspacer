---
title: Git for Windows preserves dependency junctions while returning removal success
date: 2026-09-29
promoted: false
---

# Git for Windows preserves dependency junctions while returning removal success

## Observation
Actual a1b066b2 containment Windows retained apps after git worktree remove succeeded, even after a two-second settle check. Git-for-Windows main dir.c remove_dir_recurse contains an is_mount_point(path) branch that sets kept_up and returns zero, deliberately preserving junctions and ancestor directories. Rust link_dependencies uses junction::create for ignored node_modules on Windows, explaining this result without a process-handle theory. New allocations now persist successfully linked relative paths. Removal first requires a clean Git tree and ignored/untracked entries, then validates no reparse parent and unchanged expected source target before junction::delete detaches only recorded links. No target contents or unrecorded junctions are removed. Existing actual worktree integration also verifies original dependency bytes survive. Primary reference: https://raw.githubusercontent.com/git-for-windows/git/main/dir.c (is_mount_point branch); actual Windows validation of this follow-up is pending.
