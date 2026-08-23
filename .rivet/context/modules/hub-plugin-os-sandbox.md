---
title: Hub Plugin OS Sandbox (bwrap / Seatbelt filesystem confinement)
tags: [hub, go, security, sandbox, plugins, cross-platform, sidecar]
related_paths:
  - "services/hub/internal/sandbox/*.go"
  - "services/hub/internal/plugin/sandbox_manager_test.go"
  - "services/hub/internal/integration/editor_sandbox_test.go"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Hub Plugin OS Sandbox (bwrap / Seatbelt filesystem confinement)

## Overview
This layer wraps a plugin sidecar's launch argv in an OS-level filesystem jail so a sidecar that ignores the bus and pokes the OS directly still can't tamper outside its own plugin directory. It is deliberately narrow: it confines **writes only** (write roots + a private tmp), leaves **reads fully open**, and does **not** cut network (every sidecar needs loopback to reach the hub bus). It complements — does not replace — the bus's capability scoping (`services/hub/internal/capspec`), which confines what a plugin asks workspacer to do; this confines what the raw process can do on its own.

## Key modules
- `services/hub/internal/sandbox/sandbox.go` — the whole mechanism: `Mode` (off/best-effort/enforce), `Policy{WriteRoots}`, `Result`, `Decide(mode, available) Decision`, `Wrap(command, args, Policy) Result` (dispatches by `runtime.GOOS`), `buildBwrapArgs` (Linux), `buildSeatbeltProfile`/`sbplString` (macOS).
- `services/hub/internal/sandbox/sandbox_test.go` — pure argv/profile unit tests: `TestBuildBwrapArgs`, `TestBuildSeatbeltProfile`, `TestDecide`, `TestParseMode`, `TestSbplStringEscaping`, `TestWrapReturnsRunnableCommand` — run on any platform, no bwrap/sandbox-exec binary needed.
- `services/hub/internal/sandbox/smoke_linux_test.go` — `TestBwrapReallyConfinesWrites`, a real (Linux+bwrap-only, skips otherwise) end-to-end check that a wrapped `/bin/sh` can write inside its granted root but is denied writing outside it.
- `services/hub/internal/plugin/manager.go` — the wiring: `Manager.sandboxMode` field, `SetSandboxMode(mode)`, and `sandboxSidecar(mf Manifest)` which calls `sandbox.Wrap(cmd, cmdArgs, sandbox.Policy{WriteRoots: []string{mf.Dir}})` then `sandbox.Decide(mode, res.Available)` and publishes `plugin.sandboxed` / `plugin.unsandboxed` / `plugin.sandbox.refused` events. Called from `Add()` before starting the supervisor.
- `services/hub/internal/plugin/sandbox_manager_test.go` — `TestSandboxSidecar_Off/_BestEffort/_Enforce`, exercising `Manager.sandboxSidecar` end to end against `sandbox.Decide`/`sandbox.Wrap`.
- `services/hub/cmd/hub/main.go` (L356) — the only production caller: `mgr.SetSandboxMode(sandbox.ParseMode(os.Getenv("WORKSPACER_PLUGIN_SANDBOX")))`, set once at hub startup before plugins load.
- `services/hub/internal/integration/editor_sandbox_test.go` — `TestEditorPaneTokenSandbox`, a real-bus end-to-end proof of the *complementary* layer (per-pane bus token + `${agentCwd}` path scoping in `services/hub/internal/capspec`), not the OS sandbox itself — useful contrast for "which layer stops what."

## Failure modes
- **Windows fail-closed hazard**: `Wrap`'s `runtime.GOOS` switch has no `windows` case; it falls to `default`, returning `Result{Available: false, Note: "no filesystem sandbox mechanism on windows"}`. Under `ModeEnforce`, `Decide` then returns `Refuse`, and `sandboxSidecar` returns `run=false` — the sidecar never starts, and `Manager.Add` simply skips `l.sup = supervisor.New(...)`; no error propagates to the caller beyond the `plugin.sandbox.refused` event.
- **best-effort silent downgrade**: when no mechanism is available and mode is not `off`, the sidecar still launches unconfined; the only signals are a `log.Printf` WARNING and a `plugin.unsandboxed` bus event — nothing blocks it. A consumer only watching for `plugin.loaded` would never notice the sidecar has full filesystem/network access.
- **Linux missing bwrap / macOS missing sandbox-exec**: `wrapLinux` does `exec.LookPath("bwrap")`; `wrapDarwin` does `os.Stat("/usr/bin/sandbox-exec")`. Either failure returns the original unwrapped command with `Available: false` and a `Note` string — same best-effort/enforce branching as above.
- **Empty `WriteRoots` entries**: both `buildBwrapArgs` and `buildSeatbeltProfile` skip `r == ""` roots silently (no error) — a manifest bug producing an empty `mf.Dir` would just mean *no* write root is granted (writes fail everywhere except tmp), not a wide-open sandbox — fails safe, not silently permissive.

## Gotchas
- **Writes-only, reads wide open by design**: `buildBwrapArgs` does `--ro-bind / /` then re-binds only write roots read-write; reads are never restricted. This stops tampering/persistence but explicitly does **not** stop a compromised sidecar from reading and exfiltrating arbitrary files — that boundary is intentionally left to network/process isolation the sidecar doesn't have anyway (it still needs loopback for the bus).
- **Full confinement needs no sidecar at all**: per the package doc comment and `sandbox.go`'s header, a plugin that needs stronger isolation than "confined writes, open reads" should ship as **webview-only** (no `Server`/sidecar) — the bus becomes its only door, since there's nothing left to escape. `Manager.Add` only calls `sandboxSidecar` when `mf.Server != nil`.
- **SBPL is last-rule-wins**: `buildSeatbeltProfile` relies on `(allow default)` → `(deny file-write*)` → `(allow file-write* (subpath ...))` ordering; `sandbox_test.go`'s `TestBuildSeatbeltProfile` explicitly asserts the deny appears *before* the write-root re-allow in the string — reordering these lines silently breaks confinement (macOS Seatbelt takes the last matching rule).
- **`Decide` truth table is the single source of policy**: off→always unsandboxed; enforce→sandboxed-or-refuse (fail closed), never unsandboxed; best-effort→sandboxed-if-available else unsandboxed (never refuses). Any new caller must go through `Decide`, not reimplement mode branching, or it risks silently reintroducing an enforce-mode leak.
- **Env wiring lives only in `services/hub/cmd/hub/main.go`**: `WORKSPACER_PLUGIN_SANDBOX` is read exactly once via `sandbox.ParseMode(os.Getenv(...))` before plugins load; `SetSandboxMode` is otherwise called only from tests. There's no per-plugin override in the manifest — mode is hub-global.
- **`Wrap` always returns a runnable command** (`TestWrapReturnsRunnableCommand`) — callers must still check `Result.Available` to know whether it's actually confined; a naive caller that just execs `Result.Path`/`Args` without checking `Available`/`Decide` would silently run unsandboxed on unsupported platforms.
- **`plugin.sandbox.refused` vs `plugin.sandboxed`/`plugin.unsandboxed` are the only externally visible signals** of this layer's outcome (no HTTP field on `/plugins`); anything consuming plugin state needs to subscribe to these event types on the bus to know if a loaded plugin is actually confined.
