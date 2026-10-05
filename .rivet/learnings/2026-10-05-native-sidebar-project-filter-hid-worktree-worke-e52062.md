---
title: Native sidebar project filter hid worktree workers
date: 2026-10-05
confidence: medium
related_paths:
  - apps/native/src/navigation.rs
  - apps/native/src/ui/navigation.rs
promoted: false
---

# Native sidebar project filter hid worktree workers

## Observation
Workspacer workers spawn into ~/.workspacer/worktrees/<repo>/<task> while their manager runs in the repo checkout. visible_sessions filtered each row by same_dir(cwd, project), so opening the manager's project showed only the manager (sidebar 'SESSIONS 1') even though parentSessionId was present in sessions.snapshots. Search had the same per-row problem. navigation::lineage_filter now matches a session if it or an ancestor matches, and keeps eligible ancestors of matches for context; show_screen's filter-clearing check also walks ancestors.

## Impact
Child nesting looked 'lost' whenever a project filter or search was active.
