---
title: Background task log goes blank for `log=$(mktemp); cmd > "$log"` shells
date: 2026-11-19
confidence: medium
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/session/task_redirect.rs
  - services/claudemon/src/session/background_tasks.rs
  - services/claudemon/src/daemon/api.rs
  - apps/native/src/ui/tasks.rs
promoted: false
---

# Background task log goes blank for `log=$(mktemp); cmd > "$log"` shells

## Observation
task_redirect::redirect_target refuses to expand subexpressions, so `log=$(mktemp)` leaves `$log` unknown and the redirect goes undetected. The panel then follows the CLI task file, which only gets the trailing grep/tail/rc lines when the command ends. The native tasks panel also stops polling permanently on its first error, any 404 from sessions.taskOutput (redirect file 'not written yet', 'this task has no log' before the launch tool_result arrives), until the user clicks Try again. PTY-transport Claude sessions have no background_task_list at all.

## Impact
The log panel works for some background shells and not others, depending on how the command is written and when the panel is opened.
