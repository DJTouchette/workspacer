# Desktop agent performance

Routine session history metrics are coalesced across the fleet for 250 ms.
Lifecycle changes and session end commit immediately. The batch contains only
session metrics, is replayed under the existing cross-process lock against a
freshly loaded history, and is retained for retry if the write fails. Explicit
history mutations drain earlier observations first; desktop shutdown also
flushes outstanding observations. An abrupt process crash can lose the most
recent queued metrics.

## Fleet and transcript updates

Direct Electron IPC sends compact fleet snapshots (12 recent turns) and only
sends full live history for sessions with active detail viewers. The preload
reference-counts viewers and navigation clears old demand. Full-history reads
remain available for activation and refresh. The bus/remote backends retain
their existing window/delta fold; the direct-only detail capability is deliberately
absent there. Hidden background reads do not start new bus transcript demand.

Routine renderer fleet updates publish in one 100 ms batch. New sessions,
state changes, approvals/questions, and ended sessions bypass the delay.
Stable action/directory contexts isolate hidden session chips from streaming
updates. Active panes reconcile unchanged turns and maintain incremental
user/tool/workflow indexes; content changes in earlier turns are still detected.
This retains an O(history) identity/comparison pass for full IPC detail updates;
it does not claim that active streaming is independent of transcript size.

## Startup and backend work

Worktree dependency discovery uses committed parent directories from Git HEAD,
avoiding recursive walks through ignored build artifacts. Concurrent launches
share a lookup and a 30-second result cache per canonical repository. Sources
are revalidated before linking; `discoverWorktreeNodeModules(root, { refresh:
true })` explicitly refreshes the cache. Clean-worktree/symlink checks remain.

Provider catalog requests share concurrent cache misses. The optional Codex
bundled-model probe has a two-second deadline and child cleanup; desktop catalog
requests have a 20-second deadline.

Transcript tailing permits up to eight sessions to advance concurrently, with
256 KiB read chunks. Subagent membership is rediscovered every two seconds;
unchanged files back off to five-second checks. This reduces metadata work but
can delay noticing new activity in an idle historical subagent by up to that
interval. Backlogs remain eligible to drain after session end, with an absolute
five-minute catch-up deadline so a deleted/unreadable transcript cannot retain
dead session logs forever; the normal no-backlog drain remains 30 seconds.

State-event overflow emits an explicit resync marker. The desktop bridge
reconciles authoritative states on connection and resync, protecting against
older fetches/queued events overwriting newer lifecycle state. Recovery reads
request state only so reconnect does not trigger historical usage/transcript work.
The headless brain also refreshes known session states on lag markers.

## Reproduce the history cost

From `apps/desktop`, run:

```sh
npm run bench:agent-load
```

This uses production history and compaction code with disposable synthetic
data. It compares immediate and batched observation bursts at 1, 50, and 200
stored tasks, including the final disk commit. It reports filesystem operation
counts and elapsed time. The snapshot section compares Node structured cloning
of full and compact synthetic histories; it is not an Electron IPC profile.
No agent is launched and no personal history/configuration is read.

## Startup timing logs

Search `workspacer.log` in the app's **Open logs** folder for `spawn-timing`.
Main-process entries contain `traceId`, provider, transport when specified,
stage, duration/elapsed milliseconds, and a session ID when allocated. The
same trace ID connects preflight, facade readiness, launch assets, integration,
metadata, daemon admission and total launch time. `daemon_admission` means the
daemon accepted the request, not that the provider is initialized.

Worktree logs have their own trace and separate repository inspection, Git
creation, review allocation, dependency linking and setup commands. Renderer
DevTools also logs worktree/setup, the spawn IPC round trip, and workspace state
commit. These renderer timings are in the browser console, not the main log;
workspace commit does not measure the subsequent React paint.

The daemon logs session-correlated Claude initialization and Codex app-server
spawn, HTTP readiness, WebSocket connection and thread subscription. Codex
app-server elapsed times start at server launch preparation; thread subscription
elapsed time starts at protocol setup. Daemon output is captured in the desktop
log when the desktop owns it; for an externally managed daemon, use that daemon's
logs. Native Claude PTY readiness and other providers are not separately timed.
Fixed stage names and identifiers are logged, never prompts, argv or credentials.
Operational timing records go to stderr so headless stdout stays valid JSON.

`dispatch-history-timing` reports every history transaction taking at least 50 ms,
including immediate lifecycle transitions, admission, explicit mutations, fleet
refreshes, and timer/shutdown flushes. Fields include `trigger`, queued `sessions`,
explicit `observations`, total `durationMs`, `lockWaitMs`, `writeMs`, `wrote`, and
`succeeded`. Write time includes serialization and the atomic file commit; total
time also includes loading and applying history. Failed lock acquisition is timed
too. These logs can reveal residual storage/lock contention under load.

Task Inspector and Recent Agents share one three-second poll while mounted.
Each backend read loads tasks and all requested manager inbox summaries from one
file version. Headless fleet refreshes persist changed history metrics in one
transaction, with lifecycle states durable before the refresh returns.
