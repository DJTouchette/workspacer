---
title: workspacer serve CLI (headless server launcher)
tags: [hub, cli, headless-server, process-supervision, auth-token, remote, ports]
related_paths:
  - "services/hub/cmd/workspacer/main.go"
  - "services/hub/cmd/workspacer/serve.go"
  - "services/hub/cmd/workspacer/plan.go"
  - "services/hub/cmd/workspacer/child.go"
  - "services/hub/cmd/workspacer/backoff.go"
  - "services/hub/cmd/workspacer/token.go"
  - "services/hub/cmd/workspacer/tokencmd.go"
  - "services/hub/cmd/workspacer/status.go"
  - "services/hub/cmd/workspacer/install.go"
  - "services/hub/cmd/workspacer/resolve.go"
  - "services/hub/internal/authtoken/authtoken.go"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# workspacer serve CLI (headless server launcher)

## Overview

`workspacer` (package `main` at `services/hub/cmd/workspacer/`) is the standalone launcher binary for headless server mode. `workspacer serve` resolves, launches, and supervises two sibling daemons — claudemon (the Rust session daemon) and the hub (the Go event bus, started with `--brain-scope full` so it in turn supervises a brain that supplies the whole capability surface) — wires their ports and a shared auth token together, and prints the URLs + pairing token a remote client (`/remote`, `/m`, the TUI, MCP) needs to connect. A full-scope brain + claudemon together *is* the headless server; this binary holds no product logic of its own, only launch/supervise/report. It also exposes `workspacer status` (probe what's running), `workspacer token` (mint/list/revoke capability-scoped bus tokens), and `workspacer install-cli` (put the binary on PATH). It is built by `make build-cli` (which itself depends on `build-hub` and `build-claudemon`), distinct from the desktop app's own daemon-spawning path (`apps/desktop/src/main/services/hubDaemon.ts`) — this is the CLI/server-tarball route to the same daemon pair.

## Key modules

- `services/hub/cmd/workspacer/main.go` — argv dispatch (`serve` / `status` / `token` / `install-cli` / `help`); no other logic.
- `services/hub/cmd/workspacer/serve.go` — `runServe`: parses flags, resolves binaries, mints/loads the token, refuses to double-start on already-bound ports (`probeListen`), builds the plan, starts both children, polls `/health` on each (`waitForHealth`, 20s timeout), prints the ready banner (human or `--json`), then blocks on `ctx.Done()` (SIGINT/SIGTERM) and shuts the hub down before claudemon on exit.
- `services/hub/cmd/workspacer/plan.go` — `buildServePlan` (pure function, table-tested): wires `serveOptions` into `childSpec` argv for both daemons, the loopback health-check URLs, and the `bannerInfo` client URLs. Also `advertiseHost`/`isTailscaleIPv4`/`localIPv4s` — picks which host to print in client URLs when binding a wildcard (`0.0.0.0`/`::`), preferring a Tailscale CGNAT address (100.64.0.0/10) over any other non-loopback IPv4. `renderBanner` formats the human-readable ready banner.
- `services/hub/cmd/workspacer/child.go` — `child`/`childSpec`/`startChild`: a bespoke child-process supervisor (crash-restart with backoff, SIGTERM-then-SIGKILL stop, `prefixWriter` for line-prefixed forwarded logs). Sets `WORKSPACER_PARENT_PID` env and wires a parent-death pipe (`parentR`/`parentW`) so a SIGKILL'd launcher doesn't orphan its children.
- `services/hub/cmd/workspacer/backoff.go` — `restartBackoff`/`newRestartBackoff`: pure bookkeeping — 1s base delay, doubling, 30s cap, gives up after 10 consecutive failures, resets the counter after 1 minute of healthy uptime. A direct port of the desktop's `RestartBackoff` (`apps/desktop` daemonUtils.ts).
- `services/hub/cmd/workspacer/token.go` — `configDir()` (delegates to `authtoken.ConfigDir()`) and `loadOrCreateToken`: mints/persists the host pairing token at `<config>/remote-token` (24 random bytes, base64url, file mode 0600), reusing the exact file/format the desktop app writes so pairings work interchangeably.
- `services/hub/cmd/workspacer/tokencmd.go` — `runToken`/`runTokenCreate`/`runTokenList`/`runTokenRevoke`: CLI for `services/hub/internal/authtoken`'s scoped-token store (`<config>/workspacer/tokens.json`), thin wrapper over `services/hub/internal/authtoken`'s `Mint`/`Load`/`Revoke`.
- `services/hub/cmd/workspacer/status.go` — `runStatus`: probes claudemon `/health`+`/sessions`, hub `/health` (with token), and — only if the hub is up — calls the bus method `app.getCwd` (`probeBrain`) to determine whether a brain is actually registered, since method counts alone can't distinguish the hub's built-in methods from a live brain provider.
- `services/hub/cmd/workspacer/install.go` — `runInstallCLI`/`pickInstallDir`/`installBinary`: symlinks (Unix) or copies (Windows) the running executable onto PATH; prefers `/usr/local/bin` if writable, else `~/.local/bin`; Windows targets `%LOCALAPPDATA%\workspacer\bin`.
- `services/hub/cmd/workspacer/resolve.go` — `resolveBin`/`selfDir`/`defaultWebappDir`/`exeName`: binary/dir resolution shared by `serve.go` and `install.go`. Resolution order for each sibling daemon: explicit flag → sibling of this executable (symlinks resolved via `selfDir`) → `PATH`.
- `services/hub/internal/authtoken/authtoken.go` — the scoped-token store this CLI drives: `Scope` (`view`/`triage`/`operator`), `viewMethods`/`triageMethods` allowlists (fail-closed — any method not explicitly listed is denied to scoped tokens), `Store` (mtime/size-gated read-through cache the hub uses at its dispatch point), `ConfigDir()` (the single canonical definition the CLI's `configDir()` delegates to).

## Failure modes

- **Missing daemon binaries are fatal for claudemon/hub, a soft warning for the brain.** `runServe` exits 1 with `"workspacer: claudemon binary not found next to this binary or on PATH (build it with make build-claudemon, or pass --claudemon-bin)"` (same shape for hub), but a missing brain binary only logs `"workspacer: warning: no brain binary found — the bus will have no headless capability provider"` and continues — the bus still comes up, just capability-less.
- **Port collisions refuse to start rather than killing the incumbent.** `probeListen` checks all three ports (`claudemon hook port`, `claudemon API port`, `hub port`) before spawning anything; on collision it prints `"workspacer: %s %d is already in use (%v) — is a workspacer server or the desktop app already running? Try workspacer status."` and exits 1. This is a deliberate asymmetry with the desktop app, which *does* kill stale listeners because it owns its daemons — the CLI can't know a listener isn't another deliberate `workspacer serve`.
- **Health-wait timeout (20s, `readyTimeout`) triggers a full shutdown, not a bare exit.** If claudemon or the hub doesn't answer `/health` with 200 within the window, `runServe` calls `shutdown()` (stops hub then claudemon) before returning exit code 1 — it never leaves an unhealthy half-started pair running.
- **Repeated child crashes give up loudly instead of retrying forever.** After `maxAttempts=10` consecutive failures inside a `resetAfter=1min` window, `child.run` sets `gaveUp` and logs `"[workspacer] %s keeps crashing — gave up restarting it (last error: %v)"` and returns — the supervision loop for that child stops permanently for the life of the process (there is no external retry).
- **`workspacer token revoke` requires ≥8-char prefix or the full token**, otherwise errors with `"token reference %q too short (give the full token or ≥8 leading characters)"`; an ambiguous prefix errors with `"prefix %q matches more than one token"` rather than picking one. Revoking only blocks *future* connections — a session already connected with that token keeps its grants until it disconnects (same behavior as rotating the host `remote-token`).
- **`workspacer status` distinguishes "hub down" from "hub up but token rejected" from "hub up, brain not registered."** `probeHub` returns `OK:true, Detail: "healthy (token not accepted — method count hidden)"` when the hub answers but hides its method count (bad token); `probeBrain` specifically greps the bus-call error string for `"no provider"` to report `"not registered (hub is up but no provider answered — is the brain running?)"` versus `busclient.ErrNotConnected` → `"bus unreachable (wrong token?)"`.
- **Install-over-self is a silent no-op**: `installBinary` returns nil immediately if `src == dst` (already installed, e.g. re-running `install-cli` after being invoked via the installed symlink).
- **A crash-looping child's forwarded stderr can lose its last partial line** unless `prefixWriter.flush()` runs — this is called explicitly after `cmd.Wait()` returns, so a child that dies mid-line (no trailing `\n`) still gets that line logged.

## Gotchas

- **Twin process supervisor, deliberately not shared with `services/hub/internal/supervisor`.** `child.go`/`backoff.go` reimplement crash-restart-with-backoff rather than reuse `services/hub/internal/supervisor` (used for the hub's own plugin/sidecar supervision). Two behavioral differences justify the duplication: (1) `prefixWriter` forwards child stdout/stderr to the operator's terminal — `services/hub/internal/supervisor` discards it, which would make a headless server undebuggable; (2) `restartBackoff` gives up after 10 consecutive failures (`gaveUp` flag) — `services/hub/internal/supervisor` retries forever, appropriate for backgrounded sidecars but wrong for a foreground CLI that should say "I give up" instead of silently hammering a broken binary forever. Do not merge these two supervisors without re-litigating both differences.
- **`restartBackoff` is a direct behavioral port of the desktop's `RestartBackoff`** (comment in `backoff.go` names `apps/desktop` `daemonUtils.ts` explicitly) — base 1s, doubling, 30s cap, reset after 1 minute of healthy uptime, give up after 10 attempts. If the desktop's backoff constants ever change, this Go copy will silently drift; there is no shared source of truth between the TS and Go implementations.
- **The pairing token file is a three-way shared invariant.** `<config>/remote-token` (config dir per `authtoken.ConfigDir()`, mirrored by `apps/desktop` `getConfigDir`/`configService.ts`) is read/written by: the desktop app (`apps/desktop/src/main/services/hubDaemon.ts`), this CLI's `loadOrCreateToken` (`token.go`), and the hub itself. All three must agree on the directory-resolution algorithm (Windows `%APPDATA%\workspacer`, else `$XDG_CONFIG_HOME/workspacer` or `~/.config/workspacer`) or pairings silently break for one client but not another. `services/hub/internal/authtoken.ConfigDir()` is the single canonical definition; the CLI's own `configDir()` in `token.go` just delegates to it — don't reimplement it a third time.
- **Scoped tokens (`tokens.json`) fail closed by construction.** `authtoken.viewMethods`/`triageMethods` are explicit allowlists (exact method names, no wildcard globs for scoped tiers) — any bus method added later is automatically denied to `view`/`triage` tokens until someone deliberately adds it to the list. Only `operator` scope is a wildcard (`"*"`). Anyone adding a new read-only bus method must remember to add it to `viewMethods` or scoped clients (the `/m` PWA, `/remote`) silently lose access to it.
- **The host `remote-token` has no scope record and is implicitly `operator`** — it predates the scoped-token system, so revoking or auditing scoped tokens in `tokens.json` has zero effect on it; there is no CLI command to rotate/revoke the host token itself (only `token create/list/revoke` for the scoped ones). Deleting `<config>/remote-token` and restarting `workspacer serve` is the only way to rotate it, and that invalidates every existing pairing, not just one.
- **`--brain-scope full` is the load-bearing flag that makes this a headless server rather than a bare bus.** `buildServePlan` (`plan.go`) always passes `--brain-scope full` to the hub child — this is what makes the hub supervise a full-capability brain instead of running bus-only or in `catalog` mode (the mode the desktop app uses when it owns the live capabilities itself, per `services/hub/cmd/hub/main.go`'s `--brain-scope` flag help text). Changing this constant changes the entire product meaning of `workspacer serve`.
- **`selfDir()` resolves symlinks before finding siblings**, so an `install-cli`-created symlink on PATH still discovers the real claudemon/hub/brain binaries next to the *original* binary, not next to the symlink — sibling resolution would silently fail (falling through to bare PATH lookup) if this weren't done.
- **`waitForHealth`'s probe window and `probeListen`'s port-check have a real TOCTOU gap** (both files' own comments acknowledge this: "There is a small window between the probe and the child binding for real") — accepted as a tradeoff for a clearer error message over silent EADDRINUSE crash loops, not eliminated.
- **`opts.Host` only affects the hub's bind address — claudemon is hardcoded to `127.0.0.1`** in `buildServePlan` (`claudemon.Args` always passes `--host 127.0.0.1`). Remote clients never talk to claudemon directly under any `--host` setting; they always go through the hub bus, mirroring how the desktop app talks to claudemon.
- **The banner deliberately prints the token in plaintext to stdout/stderr** (`renderBanner`'s comment: "unlike the desktop... a headless server's terminal IS the pairing surface") — this is an intentional UX choice, not an oversight; don't "fix" it by redacting the token without understanding this is how operators are expected to get the pairing credential.

## Hand-authored notes (2026-08-24) — two things `serve` never passed to claudemon

Both were the same class of omission: `buildServePlan` (`plan.go`) passed
claudemon only `serve --host --hook-port --api-port`, and everything the desktop
does around that spawn was invisible from the plan.

- **The SQLite session store was unpinned, so two stacks on alternate ports shared
  one `state.db`.** With no `--db-path`, `cli.rs` falls back to
  `store::default_db_path()`: `$XDG_DATA_HOME/claudemon/state.db`, else
  `~/.claudemon/state.db`, else the **RELATIVE** `.claudemon/state.db` under the
  process CWD. Nothing downstream supplied it (the desktop's
  `claudemonDaemon.ts` spawn omits it too) and there is no env override besides
  `XDG_DATA_HOME`. Consequence: `bootStack` refuses busy ports, so the ONLY way to
  run a second stack is to change claudemon's ports — and that second daemon then
  opened the SAME `~/.claudemon/state.db`. The two share the `sessions` and
  `events` tables, so the newcomer's boot hydration (`load_recent_sessions` →
  `store.hydrate`, which marks rows Stopped IN MEMORY only) lists the live stack's
  agents as its own resumable sessions, its clients can `claude --resume` them,
  and its `fleet.quiescence` sampler counts them. **Silent on both sides.**
  Fix: `resolveDBPath` (`serve.go`) decides before any port probe or spawn, and
  `buildServePlan` carries the answer into the argv. **The derived default MIRRORS
  `default_db_path` exactly** — desktop and serve share one session store on
  purpose (adopt-don't-kill), so relocating it would strand every install. Two
  deliberate non-copies: claudemon's relative third fallback, and Rust's
  `env::var` returning `Ok("")` for a set-but-empty `XDG_DATA_HOME` (which also
  yields a relative path) — both now REFUSE with a named fix instead of silently
  landing the DB on an ephemeral CWD. New `--claudemon-db-path` flag, REQUIRED
  whenever either claudemon port differs from its default.
  **Anything added to claudemon's argv in `plan.go` should ask "is this persistent
  state?"** — the ports were all pinned and the one file it opens was not. Keep
  `deriveClaudemonDBPath` in lockstep with `store/mod.rs`'s `default_db_path`;
  `TestResolveDBPath` is the guard.
- **`claudemon init` never ran, and the severe consequence is not missing
  telemetry — it is that a dispatched PTY worker never receives its prompt.**
  `claudemon init` is a PEER subcommand (`services/claudemon/src/cli.rs`) whose
  only caller in the whole repo was the desktop's Electron main
  (`runClaudemonInit`). Since both share one `~/.claude/settings.json`, `serve`
  inherited working hooks on any machine where the desktop had ever run, and
  registered nothing on a state dir where it had not. A PTY session is born
  `SessionMode::Unknown` and **ONLY hook events move it to Input/Responding**, and
  a spawn's `first_message` is held until the `Input` transition
  (`session/store.rs` `queue_first_message` → `schedule_pending_flush`, pinned by
  `a_pty_first_message_waits_for_the_input_transition`). So with no hooks a
  dispatched PTY worker **sits at an empty composer looking alive**; `POST
  /message` likewise only queues, and permission prompts produce no approvable
  record. Fixed by running `claudemon init --hook-port N` as a one-shot pre-flight
  inside `bootStack`, before the daemons (`servePlan.Init`), with
  `--no-claudemon-init` to opt out.
  *Counter to a scout claim: `fleet.quiescence` is NOT fooled* —
  `services/hub/internal/quiescence`'s `stateBlocker` treats `mode: "unknown"` as
  `KindSessionUnknown`, i.e. a BLOCKER, not rest. A hookless PTY session pins the
  machine awake, which is the safe direction and the reason this stayed invisible.
  Stream transport (the default, `services/hub/cmd/brain/config_defaults.json`) is unaffected:
  managed adapters call `providers::set_mode` directly.
  **When adding anything to the serve path that depends on Claude Code hooks,
  remember the desktop and the CLI share `~/.claude/settings.json` — a dev machine
  cannot reproduce the hookless case. Test against a HOME with no settings.json.**

*(Also verified while checking this: `serve` DOES refuse busy ports rather than
killing them — `serve.go`'s `probeListen` — unlike the desktop, which kills stale
listeners because it owns its daemons.)*

Image/deployment concerns for the same binary (release-artifact installs, build
stamps, boot-state records) live in `modules/fly-node-deploy.md`.
