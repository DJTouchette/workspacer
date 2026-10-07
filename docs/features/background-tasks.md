# Background tasks

Claude Code runs work beside the conversation: `run_in_background` shells (a
dev server, a watcher, a poll loop), async Agent/Task subagents, teammates,
cloud agents (`/code-review ultra`) and workflows. Workspacer shows what each
one is, its status, its log, and, when it can be proven, its process.

## Where the data comes from

Only Claude's **stream** transport reports tasks individually. claudemon's
stream driver folds these frames (captured from CLI 2.1.286; fixtures in
`services/claudemon/src/providers/testdata/claude-stream-bg-tasks*.jsonl`):

| Frame | Shape |
| --- | --- |
| `system/background_tasks_changed` | the full live set `tasks: [{task_id, task_type, description, ambient?}]`; arrives **before** the matching `task_started` |
| `system/task_started` | `task_id, tool_use_id, description, task_type, is_backgrounded, subagent_type?, spawn_depth?, prompt?, workflow_name?, owned_by_subagent?, ambient?` |
| `system/task_progress` | `task_id, tool_use_id, description, subagent_type?, usage{total_tokens, tool_uses, duration_ms}, last_tool_name?, summary?`. Agent tasks only; `description` is the latest activity ("Running Echo the letter a") |
| `system/task_updated` | `task_id, patch{status?, end_time?, description?, error?, is_backgrounded?, total_paused_ms?}` |
| `system/task_notification` | `task_id, tool_use_id, status, output_file, summary, usage?, reason?`: the terminal frame |
| background Bash `tool_result` (user frame) | `tool_use_result.backgroundTaskId` and the text `Output is being written to: <path>`, the only place the output path appears while the task runs |

Task types: `local_bash`, `local_agent`, `in_process_teammate`, `remote_agent`,
`local_workflow`. Statuses: `pending`, `running`, `completed`, `failed`,
`killed`, `stopped`. A stopped shell reports `task_updated {status: killed}`
then `task_notification {status: stopped}`; the notification wins.

Output files live at `<tmp>/claude-<uid>/<cwd slug>/<claude session>/tasks/<task id>.output`.
A shell's file is a plain file ending with `[exited with code N]` or `[killed]`.
An agent's file is a **symlink to its own transcript**
(`subagents/agent-<task id>.jsonl`), so the task id is the subagent id.

PTY sessions get none of this. They keep the plain `background_tasks` count
(from hooks) and no list.

## claudemon

`SessionState.background_task_list` (snapshot field, omitted when empty) holds
one row per task: id, type, status, description, start/end, progress usage,
summary, last tool, `toolUseId`, `subagentId` for agents, `hasOutput`, and a
`pid` only when proven. It is pure enrichment: the busy/idle derivation
(agent-type tasks hold the turn, `local_bash`/`local_workflow` stay ambient)
is unchanged. Retention is 40 rows; finished rows are kept for 2 hours, and a
running row is never evicted to keep a finished one. Driver teardown and
restarts mark leftover running rows `stopped`.

- `GET /sessions/:id/tasks/:task_id/output?offset=&max=` is a bounded read
  (default 64 KiB, max 512 KiB). It starts from the tail without `offset`,
  never splits a UTF-8 character, and sets `reset` when the file shrank. The
  file is the CLI-named path from that task's own row, accepted only as
  `…/<claude session>/tasks/<task id>.output` and validated against the
  frame's `session_id`. It is never sent on the wire and never taken from a
  caller. Reads refuse symlinks (`O_NOFOLLOW`) and anything that is not a
  regular file.
- `POST /sessions/:id/tasks/:task_id/stop` sends the CLI's own `stop_task`
  control request (the request its task UI's `x` sends) and returns the CLI's
  verdict. It never sends a signal. Only running tasks are forwarded (409
  otherwise); a non-stream session answers 501.

The hub exposes these as `sessions.taskOutput` (view and triage tiers) and
`sessions.taskStop` (triage tier, like `claude.signal`).

We deliberately do **not** declare `perTaskStopAffordance` in `initialize`.
Declaring it would make an interrupt spare running background agents and
workflows, which changes what the composer's Stop means.

## PID verdict

No frame, output file or CLI state carries a pid. A pid **is** provable on
Linux. claudemon attaches one only when all of these hold:

- exactly one **direct child** of the session's own `claude` process (the
  driver knows its pid)
- has the task's output file as **stdout** (`/proc/<pid>/fd/1`)
- **started** no earlier than 10 s before the daemon first saw the task.

Before every use (each log read samples liveness, CPU and RSS for the whole
process tree), the parent, the start time in ticks and the stdout are checked
again, so a recycled pid fails. Verified live: the CLI's shell is
`bash -c 'source <snapshot> … eval <cmd>'` with fds 1 and 2 on the `.output`
file. macOS and Windows show no pid.

Stop does not use the pid. The native `stop_task` request is the CLI's own
mechanism and needs no signal.

## Codex (follow-up)

The Codex app-server protocol (0.159.0 schema) has:

- `commandExecution` items with a `processId` (the unified-exec PTY session
  handle, a **string, not an OS pid**) and a `source` of
  `agent | userShell | unifiedExecStartup | unifiedExecInteraction`
- `item/commandExecution/outputDelta` for live output
- `item/commandExecution/terminalInteraction {processId, stdin}`.

It has no background-task list and no exit notification for agent-started
sessions. `process/exited` and `command/exec/terminate` are only for processes
the client spawned itself, so there is no stop for agent-owned sessions.
claudemon's Codex adapter consumes none of these yet.

Supporting Codex would take:

1. A live capture of a long-lived unified-exec process to learn its item
   lifecycle (does the item complete while the process lives on?).
2. Mapping `unifiedExec*` items with a `processId` to task rows.
3. Keeping `outputDelta` per item in a bounded claudemon ring, served through
   the same `taskOutput` route (there is no output file).
4. Leaving liveness and stop unsupported until the protocol offers them. An
   OS pid would need a different proof, because a PTY child has no
   task-unique stdout file to anchor it.

## Clients

Native (`apps/native`): a title-capsule chip with the running count; a panel in
the file viewer's right-side slot (docked or sheet) listing tasks; a live,
auto-following log for shells and workflows; agents open their child
conversation; Stop with confirmation. See `apps/native/README.md` →
Background tasks.
