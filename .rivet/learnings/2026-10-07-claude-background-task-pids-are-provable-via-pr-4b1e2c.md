---
title: Claude background-task PIDs are provable via /proc fd/1; stop uses the stream stop_task request
date: 2026-10-07
confidence: high
related_paths:
  - services/claudemon/src/session/background_tasks.rs
  - services/claudemon/src/providers/claude_stream.rs
promoted: false
---

# Claude background-task PIDs are provable via /proc fd/1; stop uses the stream stop_task request

## Observation
No Claude stream frame carries a pid (CLI 2.1.286). A `run_in_background` shell is a
DIRECT child of the claude process (`bash -c 'source <shell-snapshot> … eval <cmd>'`)
whose fd 1 and 2 are `…/<claude session>/tasks/<task_id>.output`. That makes the pid
provable: unique direct child + stdout readlink == the task's file + start time after
the task appeared; re-check parent/start-ticks/fd before each use. The stream control
protocol has `{"subtype":"stop_task","task_id"}` (what the CLI's own task UI sends);
verified it kills the shell and emits task_updated{killed} + task_notification{stopped}
before the empty success. Do NOT declare `perTaskStopAffordance` in initialize: it makes
`interrupt` spare background agents/workflows.

The output path appears while running only in the background Bash tool_result text
(`Output is being written to: <path>`, with `tool_use_result.backgroundTaskId`);
task_notification.output_file arrives at the end. local_agent outputs are symlinks to
`subagents/agent-<task_id>.jsonl` (task id == subagent id).
