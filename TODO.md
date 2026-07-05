# TODO — gap backlog (from the 2026-07-04 five-agent audit)

Items *not* covered by the July 2026 fix pass (spawn-path extraction, managed
error surfacing, classifier/items removal, TUI provider parity). Grouped by
theme, roughly ordered by severity within each group.

## Security (matters the moment remote share / plugins are exposed wider)

- [ ] **SECURITY.md backlog** — 10 acknowledged findings shipped by design:
      permissive CORS on claudemon mutation/spawn/git endpoints,
      hub bus `InsecureSkipVerify` origin bypass, `register` capability hijack,
      plugin-install arbitrary-command RCE + decompression bomb, unallowlisted
      `fs.read`/`fs.write` hub capabilities, session path traversal. Revisit
      before any exposure beyond Tailscale.
- [ ] **capspec path-scoping is a hand-listed 7-method allowlist**
      (`services/hub/internal/capspec/capspec.go:16-24`) — any new path-bearing
      capability gets zero FS confinement until manually added, and no test
      guards the list. Add a registration-time check or a test that fails when
      a path-bearing capability lacks a spec.
- [ ] **Sidecar sandbox confines writes only** — reads + network wide open
      (`services/hub/internal/sandbox/sandbox.go:7-16`); Windows sidecars run
      fully unconfined under `best-effort` mode.
- [ ] **MCP facade endpoints unauthenticated** (`services/hub/cmd/mcp/main.go:35,65`)
      — safe only because of the loopback default binding; `-addr` can bind
      anywhere with nothing gating callers.
- [ ] **Remote share is single-secret / full-host** — no TLS, token-in-URL,
      no per-client privilege separation, no auth rate-limiting
      (acknowledged in `docs/remote-sharing-security.md §5`).
- [ ] Plugin `install` build command runs unconfined
      (`services/hub/internal/plugin/install.go:294-305`); tarball extraction
      bounded only by a 60s timeout.

## Correctness / robustness

- [ ] **Codex model-switch during boot silently dropped** — HTTP 200 returned,
      then `codex.rs:674` logs "requested before the thread was joined —
      ignored". Queue it and apply after join, or return an error.
- [ ] **Hub supervisor hot-loops** — fixed 1s restart backoff, no cap
      (`services/hub/internal/supervisor/supervisor.go:58-63`); health-check
      failures are observability-only.
- [ ] `terminate_managed` leaks the yolo handle — removes
      `managed_inputs/decisions/model` but not `managed_yolo`
      (`services/claudemon/src/session/store.rs`, terminate path).
- [ ] Managed approval cards carry no tool input — `Pending::Approval { raw:
      Value::Null }` (`providers/mod.rs`), so the GUI can't show the payload
      for Codex/OpenCode/Pi approvals the way it can for Claude hook approvals.
- [ ] Pane-token registration race (`services/hub/internal/plugin/manager.go:230-233`)
      — register outside the mutex; concurrent Remove can leave a
      registered-but-untracked token.
- [ ] Hub token-persist write errors swallowed (`hubDaemon.ts:119-121`,
      `manager.go:283`) — a restart mints a new token and silently invalidates
      saved share/webview URLs.
- [ ] `fileWatchService.ts:87` drops watcher errors to keep the watcher alive —
      errors invisible.
- [ ] claudemon's `/sessions` JSON carries no `provider`/managed field —
      clients must guess (the TUI tracks providers only for sessions it
      spawned itself; adopted managed sessions default to claude). Add a
      `provider` field to the session payload.
- [ ] No hub capability for `GET /providers/:provider/models` — provider
      model listing is REST-only, unreachable for pure bus clients.
- [ ] OpenCode/Pi lack live model + permission-mode switch (409) — needs
      `register_managed_model_switch` / `register_managed_yolo` equivalents per
      provider, or documented as a permanent capability cliff in providerCaps.

## Web / remote parity

- [ ] `webBackend.ts` HUB-TODO stubs (Phase 3): plugin administration
      (list/install/inspect/remove/setEnabled/settings, `:250-265`),
      `providerListModels`/`providerCheckAll` (`:144-145` — Spawn dialog can't
      list provider models on web), `onTerminalExit`, workflow agent
      transcript/conversation, `onLibraryChanged` auto-refresh,
      `importChromeCookies`, `openLogsFolder`.
- [ ] `analytics.*` has no headless provider — `cmd/brain` never registers
      analytics capabilities (`services/hub/cmd/brain/handlers.go:34-105`), so
      `--brain-scope full` without the desktop app returns "no provider".
- [ ] Plugin settings-value persistence on the hub is schema-only — validation
      exists, but no Go-side storage/delivery of values and no `wks-settings`
      handler.

## Test coverage

- [ ] claudemon HTTP surface: ~30 handlers in `daemon/api.rs` have one test
      between them; `daemon/spawn.rs`, `wrapper_ws.rs`, `hook.rs` untested.
- [ ] Desktop main: `hubCapabilities.ts` and the four claudemon bridges have
      zero tests (this is how the spawn drift went unnoticed).
- [ ] Renderer: no pane/component tests for `App.tsx`, `ClaudePane.tsx`, or the
      composer/send pipeline.
- [ ] TUI: `app/input.rs` (1350-line key dispatcher) has no test module.
- [ ] Stale Playwright E2E (`claudePane.test.ts`) posts an old hook shape and
      isn't run in CI.

## CI / tooling / release

- [ ] No lint/format enforcement in any language — no eslint/prettier,
      no rustfmt/clippy config (`-D warnings` off, 27 tolerated warnings),
      no golangci. Pick and enforce per language.
- [ ] `make test` skips claudemon entirely (`Makefile:62`); local `test-hub`
      uses `-race` while CI doesn't — align both.
- [ ] No `typecheck` npm script in apps/desktop (CI calls tsc directly).
- [ ] **macOS builds unsigned + un-notarized** (`release.yml:54` disables
      signing) — Gatekeeper-blocked for normal users.
- [ ] Windows Azure Trusted Signing plumbed but pending manual identity
      validation + cert profile (`docs/windows-code-signing.md`).
- [ ] No auto-update channel (`--publish never`, no electron-updater).
- [ ] CI uses Node 20 while README/mise pin Node 22 — align.

## Docs drift

- [ ] `EDITOR-FEATURES.md` describes the removed in-app CodeMirror editor —
      rewrite around the terminal-engine `$EDITOR` pane + editor plugin, or delete.
- [ ] `docs/production-inventory.md` is stale (2026-06-13): predates `cmd/brain`
      and the capspec authz layer; calls per-method tokens "allow-all
      half-built" (now enforced), says MCP facade has 10 tools (~40), and
      self-contradicts on daemon auto-restart and signal delivery.
- [ ] `README.md:34` claims `make dev` enables remote sharing — it's
      `make dev-share`; README also says Node 22 while CI uses 20.
- [ ] `chromeCookieImport.ts` header comment implies macOS/Linux unsupported —
      only the DPAPI *fallback* is Windows-only; the default CDP path is
      cross-platform. Comment tweak.

## Cleanup

- [ ] Delete dead `components/claude/InlineFilesSection.tsx` (imported nowhere;
      superseded by `InlineWorkLog.tsx`/`ChangedFilesCard.tsx`).
- [ ] `resolveAttention.ts:3` references "(Phase 2) Fleet Deck card
      quick-actions" that were never wired.
- [ ] `docs/tui-parity.md` refresh beyond the Phase 8 entry: record the hub-bus
      transport, neovim layer, and shipped content search.
