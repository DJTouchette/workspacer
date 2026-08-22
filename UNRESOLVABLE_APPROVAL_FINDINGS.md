# Unresolvable approval + lying session state — reproduction findings

Working notes for the two defects in the project brief's `## Now` entry
("BUG — UNRESOLVABLE APPROVAL"). Written down as they were established so the
evidence survives independently of any one session's context.

## 1. The manager's parser hypothesis is WRONG

Hypothesis handed over: *"the out-of-roots denial prints a prompt shape
claudemon's PTY parser does not match"*.

There is **no PTY parser for approvals at all**. Workers spawn on the default
`claude.transport: "stream"` (`configDefaults.generated.ts:182`), i.e. the
headless stream-json managed adapter in
`services/claudemon/src/providers/claude_stream.rs`. Approvals arrive there as
`control_request` / `can_use_tool` frames on stdout — structured JSON, not
rendered text. Nothing greps the PTY.

**Reproduced directly against the real CLI** (`claude 2.1.237`) with a
stand-in driver that speaks the exact argv `build_argv()` produces
(`--print --input-format stream-json --output-format stream-json
--include-partial-messages --verbose --permission-prompt-tool stdio
--session-id … --permission-mode default`), from
`cwd = ~/.workspacer/worktrees/workspacer/<a worktree>`:

| target read | CLI behaviour |
| --- | --- |
| `~/Work/worky/workspacer/.workspacer/brief.md` (the "approvable" control) | `control_request` `can_use_tool`, `decision_reason: "Path is outside allowed working directories"`, `decision_reason_type: "workingDir"` |
| `~/.workspacer/brief.md` (the "hangs" case) | **byte-for-byte the same frame shape**, same `decision_reason`, same `decision_reason_type` |

Both are "outside allowed working directories" — the in-repo control was never
*inside* the worker's cwd either, it was just a different out-of-roots path.
**The CLI does not distinguish them.** So the divergence is downstream of the
CLI, in how workspacer folds that one frame shape into session state, and it is
a race/state bug rather than a prompt-shape bug.

## 2. Where the `can_use_tool` handshake breaks — two independent clobbers

For a stream-transport Claude session the desktop learns about approvals over
**two independent SSE connections** to claudemon, with no ordering guarantee
between them:

* `/events` → `claudemonEventBridge` → `claudeSessionStore.applyManagedMode()`
  → sets `pendingApproval` + `ambientState: 'waiting_approval'`.
  Fed in-process by `claude_stream.rs`'s `surface_approval`.
* `/hooks/stream` → `claudemonHookBridge` → `claudeSessionStore.handleHookEvent()`
  → `sessionStore/hookEventRouter.applyHookEvent()`.
  Fed by the CLI's own `PreToolUse`/`PostToolUse` hooks (a `curl` subprocess →
  claudemon hook port → rebroadcast), so it is the slower, laggier path.

### Clobber A — desktop: hooks null the approval card but not the state

`hookEventRouter.applyHookEvent` is careful to leave `ambientState` alone for
stream sessions (`hooksOwnAmbient = session.transport !== 'stream'`), because
the daemon owns that state machine. But `PreToolUse` (line 114) and
`PostToolUse` (line 153) both do an **unconditional**
`session.pendingApproval = null` regardless of transport.

So a hook frame that lands after the `/events` frame leaves exactly the
reported shape: `ambientState: 'waiting_approval'` with `pendingApproval: null`
— a session the manager can see is blocked and cannot act on, because
`agents.list` / `sessions.snapshot` read `pendingApproval` from this store.

### Clobber B — claudemon: any `Busy` update overwrites a parked approval

`providers/mod.rs::apply_updates`:

```rust
if let Some(mode) = new_mode {
    if mode != *cur_mode || mode == SessionMode::Approval {
        store.set_managed_mode(session_id, mode, pending);  // pending == None for Busy
        *cur_mode = mode;
    }
}
```

`AgentUpdate::Busy` is produced by `translate()` for `system`/`status:requesting`
(claude_stream.rs:113) and `stream_event`/`message_start` (:125). Neither is
guarded against a parked approval, so one arriving while `cur_mode ==
Approval` sets mode `Responding` **and** `pending: None` — the session then
reports `streaming` while the CLI is still blocked on an unanswered
`can_use_tool`, and the approval card is gone. That is defect 2 *and* a second
route into defect 1.

The two `background_tasks_changed` paths right below in `claude_stream.rs`
(:1616, :1637) are explicitly guarded for exactly this reason ("Guarded to
Input so a parked approval/question is never clobbered"). `apply_updates` is
the same hazard with no guard, and it is shared by *every* managed provider
(codex/opencode/pi), not just claude.

## 3. Third, independent defect: PTY sessions get no approval record at all

`session/state.rs:53` claims `PermissionRequest` is "NOT a real registerable
hook — internal / forward-compat only", so `HookEventKind::REGISTERABLE` omits
it and `daemon/init.rs::HOOK_EVENTS` never installs it. But `PermissionRequest`
**is** a real Claude Code hook event today (it is present in the live
`~/.claude/settings.json` wired to another tool), and it is the *only* thing
that can set `SessionMode::Approval` for a PTY-transport claude session
(`state.rs:684`). A PTY worker therefore never produces an approvable record —
the same unresolvable hang by a different route.

## Repro harness

`/tmp/wks-repro/driver.py <cwd> <prompt> [--answer allow|deny|none]` — spawns the
real CLI with claudemon's argv under an isolated `CLAUDE_CONFIG_DIR`
(`/tmp/wks-repro/cfg`, credentials symlinked, no hooks) so nothing lands in the
live daemon, and logs every frame with timestamps.
