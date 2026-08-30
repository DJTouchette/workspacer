---
title: Hub Process Supervision & Death-Coupling (supervisor, parentwatch, jobobject)
tags: [hub, go, process-lifecycle, supervisor, shutdown, cross-platform]
related_paths:
  - "services/hub/internal/supervisor/*.go"
  - "services/hub/internal/parentwatch/*.go"
  - "services/hub/internal/jobobject/*.go"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Hub Process Supervision & Death-Coupling (supervisor, parentwatch, jobobject)

## Overview
Three coordinated Go packages keep the hub's supervised child tree (brain capability provider, plugin sidecars) alive under normal operation and cleanly dead when any ancestor dies unexpectedly. `supervisor` owns spawn/health-check/restart/graceful-stop of one child process. `parentwatch` runs inside each spawned child and self-exits when its launcher dies. `jobobject` is a Windows-only OS-level backstop that kills the whole descendant tree when the hub process itself dies, covering grandchildren neither of the other two mechanisms reaches.

## Key modules
- `services/hub/internal/supervisor/supervisor.go` — `Supervisor.run` manage loop: `exec.CommandContext` spawn, `Running`→`Healthy`/`Unhealthy` HTTP polling (`healthLoop`/`ping`, 200 == healthy), exponential backoff (`nextBackoff`), parent-death pipe setup, `mergeEnv`.
- `services/hub/internal/supervisor/terminate_unix.go` — `terminate()` sends `SIGTERM` via `p.Signal`.
- `services/hub/internal/supervisor/terminate_windows.go` — `terminate()` calls `p.Kill()` directly (`Signal(SIGTERM)` errors on Windows).
- `services/hub/internal/parentwatch/parentwatch.go` — `Watch(onParentExit func())`, gated on `WORKSPACER_PARENT_PID`; races stdin-EOF vs pid-poll triggers via `sync.Once`.
- `services/hub/internal/parentwatch/parentwatch_unix.go` — `watchParent`/`parentAlive` poll `syscall.Kill(pid, 0)` every `pollInterval` (1s); `EPERM` = alive, ambiguity biased toward "alive".
- `services/hub/internal/parentwatch/parentwatch_windows.go` — `watchParent` opens a pinned `SYNCHRONIZE` handle once via `OpenProcess`, then `WaitForSingleObject` blocks until signaled.
- `services/hub/internal/jobobject/jobobject.go` — package doc explaining the Windows orphaned-grandchild problem this solves.
- `services/hub/internal/jobobject/jobobject_windows.go` — `Confine()` creates a `KILL_ON_JOB_CLOSE` job, assigns `CurrentProcess()`, never closes the handle.
- `services/hub/internal/jobobject/jobobject_unix.go` — `Confine()` no-op (process-group + parentwatch already cover Unix).
- `services/hub/cmd/hub/main.go` — wires all three: `jobobject.Confine()` (line ~163), `parentwatch.Watch(cancel)` (~173), `supervisor.New(...)` for the brain (~520, publisher = the event bus `b`).
- `services/hub/internal/plugin/manager.go` — `supervisor.New(...)` per plugin sidecar (~367), with `HealthURL` set and `m.pub` as publisher; stops the previous supervisor on reload to avoid a goroutine leak.
- `services/hub/cmd/brain/main.go`, `services/hub/cmd/mcp/main.go` — child-side `parentwatch.Watch(cancel)` calls.

## Failure modes
- `cmd.Start()` failure (bad path/missing binary) is treated as the classic startup hot-loop: state → `Crashed`, emits event, backs off via `nextBackoff` without resetting — same path as a post-start crash.
- Unexpected exit mid-run: if `time.Since(startedAt) >= resetAfter` (default 30s), backoff resets to `base` before escalating again; otherwise it doubles from wherever it left off, so a fast crash loop keeps escalating toward `maxRestartWait` (default 30s cap).
- `nextBackoff` guards `next <= 0` (overflow from doubling) by clamping to `limit`, so a pathological doubling never yields a negative/zero sleep.
- `ctx.Err() != nil` after `cmd.Wait()` means the stop was intentional (`Stop()` called `cancel()`): state goes to `Stopped`, no restart, no `Crashed` event — this branch must be checked before treating the exit as a crash.
- Health polling and restart backoff are independent: `Unhealthy` never triggers a restart by itself — only the process actually exiting (`cmd.Wait()` returning) enters the restart path.
- On Windows, `terminate()` skips SIGTERM entirely and calls `Kill()` because `Signal(syscall.SIGTERM)` errors there; before this fix `WaitDelay`'s 5s force-kill applied serially to every sidecar on shutdown.
- `parentwatch`'s Unix `parentAlive` errs toward "alive" on any syscall result other than success/EPERM, so a spurious watchdog exit requires either the pid to genuinely disappear (ESRCH) or the stdin-EOF trigger to fire independently.

## Gotchas
- `Supervisor.run` opens `os.Pipe()` exactly once (guarded by `s.parentR == nil`) and MUST keep `parentW` as a struct field (`s.parentW`) — otherwise `os.File`'s GC finalizer can close the write end early, causing every child to see a premature stdin EOF and self-exit via `parentwatch`.
- `parentR` is handed to *every* spawned generation of the child as `cmd.Stdin`, reused across restarts — it is not reopened per-restart.
- `Start()` is idempotent: it checks `s.done` and, if the channel is still open (previous run not finished), does nothing; only proceeds if `s.done` is nil or already closed. Callers cannot rely on `Start()` returning any signal of "already running."
- `New(spec, pub)` — `pub` (the `Publisher`) may be `nil`; `emit()` checks `s.pub == nil` and skips publishing rather than panicking. Both `services/hub/internal/plugin/manager.go` (passes `m.pub`) and `services/hub/cmd/hub/main.go` (passes the bus `b`) currently pass non-nil, but nil is a supported/tested path.
- `parentwatch.Watch` is a strict no-op when `WORKSPACER_PARENT_PID` is unset — this is intentional so a manually-run daemon from a terminal (TTY/`/dev/null` stdin, no launcher) never self-exits spuriously; don't "fix" this by defaulting the env var.
- The Windows pinned-handle approach in `services/hub/internal/parentwatch/parentwatch_windows.go` exists specifically because Windows recycles PIDs aggressively and re-opening the pid on every poll would race PID reuse (old implementation bug); the fix is to `OpenProcess` once up front and wait on that handle object, which stays valid regardless of pid reuse.
- `jobobject.Confine()`'s job handle is *intentionally* never closed — closing it would defeat the mechanism, since job death (triggered by process exit) is what fires `KILL_ON_JOB_CLOSE`. Nested jobs work since Windows 8, so this composes with an outer IDE/service-wrapper job.
- Two twin structures must stay in sync when adding a new supervised process type: the `Spec` struct's defaulting methods (`healthPeriod`/`restartWait`/`maxRestartWait`/`restartResetAfter`) and the per-callsite `supervisor.New(supervisor.Spec{...})` construction in `services/hub/internal/plugin/manager.go` / `services/hub/cmd/hub/main.go` — omitted fields silently fall back to defaults (2s/1s/30s/30s).
- `mergeEnv` overrides same-key entries in `base` rather than appending duplicates, because duplicate `KEY=val` resolution is platform-dependent; any code adding env vars to a `Spec.Env` should rely on override semantics, not append-and-hope.

## Hand-authored notes (2026-08-21) — hot-swapping the hub binary in a dev build is just SIGTERM

In an `npm run dev` desktop build the hub is an **app-OWNED child of Electron
main**. `hubDaemon.ts`'s `launch()` registers
`child.on('exit') → scheduleRestart(bin)`, guarded only by `intentionalStop` (set
by `stopHub()`/quit); `scheduleRestart` uses `RestartBackoff`
(`main/lib/daemonUtils.ts`: base 1s, ×2, cap 30s, 10 attempts / 60s reset) and
calls `launch(bin)` again, which **re-reads the binary from disk and
RECONSTRUCTS argv from config** — so a plain `kill -TERM <hubpid>` swaps a rebuilt
`services/hub/hub` in with byte-identical flags. Measured live: new listener on
:7895 ~3s later.

The process-tree facts that make this safe: `claudemon` (:7890/:7891) and the
`mcp` bridge (:7897) are **SIBLINGS** of the hub — both children of Electron main,
not of the hub — so agent sessions are untouched, and the mcp bridge reconnects
itself via `services/hub/internal/busclient` (200ms→5s backoff, subscriptions re-sent). Only
the hub's own children die and get respawned: the `brain` provider and the bwrap
plugin sidecars (`--die-with-parent`).

**This is the ONLY way to ship a `/m` PWA change**: `mobile.html`, `sw.js`, the
manifest and the icons are `go:embed`-ed in `services/hub/cmd/hub/remote.go` with no dev-mode
disk fallback, so a running hub serves its build-time copy forever. Anyone who
rebuilds the hub and then refreshes `/m` sees the OLD PWA and concludes the build
did not take.

**Check the hub's PPID first.** PPID == Electron main ⇒ app-owned ⇒ `kill -TERM`
and wait ~3s. If `workspacer serve` started it, `startHub()` **ADOPTED** it (the
health probe wins before the spawn path) and there is no supervision — killing
that hub leaves nothing to restart it.

Verify the swap, don't assume it: `md5sum /proc/<newpid>/exe services/hub/hub`
must match (the stale process's `/proc/<pid>/exe` reads `(deleted)`), then
`diff <(curl -s http://127.0.0.1:7895/m) <(git show HEAD:services/hub/cmd/hub/mobile.html)`
should be empty. PWA assets live at ROOT paths (`/manifest.webmanifest`,
`/sw.js`, `/icon-192.png`), not under `/m/`. Bus smoke-test frames use `op`, not
`type`: `{"op":"call","id":"1","method":"agents.list"}`.
