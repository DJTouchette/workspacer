---
title: Hub Process Supervision & Death-Coupling (supervisor, parentwatch, jobobject)
tags: [hub, rust, process-lifecycle, supervisor, shutdown, cross-platform]
related_paths:
  - "services/hub-rs/src/plugins/supervisor.rs"
  - "services/hub-rs/src/plugins/windows_job.rs"
  - "services/hub-rs/src/services/owned_process.rs"
  - "services/hub-rs/src/cli/parent.rs"
  - "services/hub-rs/src/backend.rs"
  - "apps/desktop/src/main/lib/daemonUtils.ts"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Hub process supervision and parent death

## Current Rust ownership

Plugin sidecars use `services/hub-rs/src/plugins/supervisor.rs` and
`windows_job.rs`; bounded command execution uses `services/owned_process.rs`.
`cli/parent.rs` owns declared-parent/stdin monitoring, and `backend.rs` owns
embedded shutdown. Standalone services no longer run a sibling Go brain child.
Electron's `daemonUtils.ts` remains a separate real process owner. Current
checks include Rust `--test plugin_manager`, `--test backend_owner` and CLI
ownership tests. Use `make build-cli` from the repo root to rebuild the current
executable. The Go packages, restart-loop details and commands below are the
historical implementation; consult the Rust owners for exact current limits.

Historical execution, when deliberately requested, uses the separate pinned
checkout described in [scripts/reference/README.md](../../../scripts/reference/README.md).
This crosswalk does not certify platform or release gates.

## Historical Go ownership

`services/hub/internal/supervisor` manages a direct child process for the
hub: plugin sidecars and, in appropriate launch modes, a brain provider.
`parentwatch` is child-side cooperation with the launcher. Windows `jobobject`
is an additional OS mechanism for descendants. These are separate mechanisms,
not a guarantee that every arbitrary descendant on every OS dies with its parent.

`workspacer serve` also has its own launcher child loop in
`services/hub/cmd/workspacer/child.go`. Electron owns another supervision path in
its daemon services. Adoption by Electron means Electron does not own that
process’s restart; its real owner may still be `workspacer serve` or another
service manager. See [serve CLI](workspacer-serve-cli.md).

## Supervisor lifecycle

`Supervisor.Start` is idempotent while its run channel remains open. `Stop`
cancels and waits. Spawn failure or unexpected child exit emits Crashed and
backs off; intentional cancellation emits Stopped and does not restart.
Default delays are one second initially, doubling to 30 seconds, reset after
30 seconds of uptime. Overflow is clamped rather than producing a zero/negative
sleep. The internal supervisor retries; the CLI’s separate loop has a finite
restart budget, so do not treat those loops as interchangeable.

Health polling defaults to two seconds, and HTTP 200 means healthy for that
probe. An unhealthy response does not itself restart a still-running child.
A health response proves only what that endpoint checks, not that the process
completed every application-level initialization step.

The run loop creates a parent-death pipe once and reuses its read end as child
stdin. Retaining the write end on the supervisor prevents a garbage-collection
close from masquerading as parent death. Pipe creation is best-effort; failure
leaves the other shutdown mechanisms rather than preventing every spawn.
`mergeEnv` replaces duplicate keys rather than relying on OS-specific duplicate
environment lookup. The launcher supplies WORKSPACER_PARENT_PID.

By default child output is discarded. `InheritOutput` forwards stdout/stderr;
`LogLines` with a publisher emits line events. Nil publisher is supported.
Plugin reload stops the previous supervisor before starting its replacement to
avoid overlapping sidecars using the same identity/directory.

## Platform shutdown behavior

On Unix the direct-child cancellation path sends SIGTERM; on Windows it calls
Kill because that os.Process signal operation is unsupported. The supervisor
sets a five-second WaitDelay for a lingering process/pipe. Neither operation by
itself is a universal process-group cleanup promise.

`parentwatch.Watch` is disabled when WORKSPACER_PARENT_PID is absent. When set,
it starts a stdin drain whose termination fires the callback once; a valid
positive PID also enables the platform watcher. Invalid PID text leaves only
the stdin path. A sync.Once combines triggers.

- Unix probes signal 0 every second. Success and EPERM mean alive; **all other
  errors mean not alive in the current implementation**. The older comment
  claiming every ambiguous error errs toward alive is not what the code does.
- Windows opens a SYNCHRONIZE process handle once and waits on that original
  process object, avoiding PID-reuse confusion. Any OpenProcess failure invokes
  the exit callback; it is not separately classified as proven process death.
  The wait result/error is not distinguished before firing either.

A plugin only cooperates with parentwatch if its code follows that protocol.
Enabled plugin processes are not launched inside the old bwrap sandbox.
On non-Windows, jobobject.Confine is a no-op and supplies no additional cleanup.

Windows Confine creates and assigns a kill-on-job-close object. On success its
handle is intentionally held for process lifetime: closing it early would fire
cleanup early, not harmlessly disable the mechanism. Creation/configuration/
assignment can fail and return an error; callers log and continue. Keep that
limitation visible rather than claiming the job is always installed.

## Historical rebuild notes and retained asset lifecycle

`mobile.html`, `remote.html`, service-worker and PWA assets are embedded in the
hub binary. Rebuilding source without replacing/restarting the actual listener
still serves the old copy. From `services/hub`, `go build -o hub ./cmd/hub`
produces the local executable; a multi-package build alone is not the same step.

Identify the live process owner and binary before restarting through that owner.
An owned child can restart after an unexpected exit, subject to its restart
budget/backoff; a standalone process may have no owner to restart it. New argv
is reconstructed from current configuration, so it need not be byte-identical
to the old launch. Check health and served content afterwards, rather than
relying on a historical three-second measurement.

## Historical Go verification

Run `go test ./internal/supervisor` and the launcher child tests from
`services/hub`. The parentwatch/jobobject packages have platform-specific source
but no equivalent live-platform coverage in that supervisor suite. Cross-compiling
checks API/build compatibility only; testing real parent loss, inherited handles
and descendant cleanup requires the relevant OS and process fixture.
