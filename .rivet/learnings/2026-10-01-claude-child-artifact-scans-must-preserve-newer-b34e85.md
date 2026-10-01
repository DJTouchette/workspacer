---
title: Claude child artifact scans must preserve newer hook observations
date: 2026-10-01
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/claude_subagents.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

# Claude child artifact scans must preserve newer hook observations

## Observation
A confined async child artifact scan starts from a session snapshot; while it awaits filesystem IO, SubagentStop or child tool hooks can update the current row. Replacing the row from the older scan would reopen a finished child or overwrite newer activity. Artifact merge now fences the session telemetry epoch and compares expected child rows, retaining changed live metadata while merging reported usage. Parent Input cannot close Claude children; final child end_turn or SubagentStop provides completion evidence. Unknown artifact start is0 and usage absence stays distinct from reported0.

## Impact
Without fencing the native card can show stale running status, stale activity, or a fabricated completion duration.

## Recommendation
Keep concurrent hook/artifact merge, idle-parent detached-child, and zero-versus-absent telemetry regressions.
