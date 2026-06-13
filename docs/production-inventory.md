# Workspacer — Production-Readiness Feature Inventory

> Compiled 2026-06-13 from a full read of all four components after the
> `apps/` + `services/` monorepo refactor. Status legend:
>
> - **WORKING** — complete and wired end-to-end.
> - **HALF-BUILT** — partial / placeholder / TODO / experimental; not shippable as-is.
> - **DEAD** — orphaned code, no live consumer (imported only by other dead code or tests).
> - **NOT BUILT** — referenced/configured but no implementation.
> - **UNCLEAR** — needs a runtime check to confirm.
>
> The far-right column of each table is the **keep/drop decision** — fill in as we triage.

---

## 0. The big picture (read this first)

Three structural facts dominate the keep/drop decision:

1. **The product pivoted away from `docs/v2-spec.md`.** That spec describes an
   "inbox of decisions" (L1/L2/L3 zoom, snooze, summarizer, supervisor). Per
   `next-features.md`, the inbox model was **dropped on 2026-06-01** in favor of
   the per-agent workspace model. The spec is now part-aspirational.

2. **An entire backend subsystem is orphaned: the claudemon classifier + `/items`
   inbox.** claudemon fully implements (with 16 passing tests) a classifier that
   detects `NeedsInput / Error / Stuck / Done`, writes priority-ranked items to
   SQLite, and serves them over `/items`, `/items/stream`, `/items/:id/action`.
   **Nothing consumes it.** The only client chain — `claudemonItems.ts` →
   `ItemDetailOverlay.tsx` — is dead (imported only by a test). wks-tui reads
   `/sessions`, not `/items`. This is the single biggest keep/drop question.
   The v2-spec SQLite schema is largely dead alongside it: the
   `pending_decisions`, `asks`, and `events_fts` (transcript search) tables are
   all created but never read or written.

3. **There are effectively TWO inbox/triage systems.** The *live, working* one in
   the renderer (`InboxDrawer` + `AttentionContext`, fed from live
   `ClaudeSessionSnapshot` ambient state) is unrelated to the orphaned classifier
   pipeline above. We are paying to maintain both halves of a feature that only
   connects on one side.

Everything else is mostly healthy. Decisions concentrate in: the orphaned
classifier/items stack, a handful of dead renderer files, two "Phase 2"
placeholder panes, no-auto-restart daemon supervision, and platform gaps
(macOS/Linux cookie import, tray/overlay icons).

---

## Decisions taken (2026-06-13)

| Decision | Outcome | Status |
|---|---|---|
| Classifier + `/items` + SQLite inbox | **Park as experimental** — kept, tagged EXPERIMENTAL/PARKED in `classifier/mod.rs`, `store/items.rs`, `api.rs`; not wired to any UI | ✅ done |
| Dead renderer client chain (`claudemonItems`, `claudemonSessions`, `ItemDetailOverlay`) | Deleted (dead regardless of the park) | ✅ done |
| Other dead renderer files (`WorkingTimer`, `terminalQueries`, `ScrollIndicator`) | Deleted | ✅ done |
| NotesPane | **Finished** — real per-agent markdown scratchpad, persists with the session | ✅ done |
| AgentPane placeholder + `'agent'` pane type | Removed | ✅ done |
| `docs/v2-spec.md` | Deleted (superseded) | ✅ done |
| claudemon/hub auto-restart on crash | Implemented (exponential backoff supervision) | ✅ done |
| SIGTERM/SIGKILL delivery to sessions | Implemented (`pty::signal_child`, real POSIX signals) | ✅ done |
| Pre-existing renderer typecheck errors (KeybindingsConfig, createTerminal decl) | Fixed in passing | ✅ done |

**Still open (not in this pass):** macOS/Linux Chrome cookie import; tray/overlay
icon; git merge / `review_diff` next-action wiring; stale E2E `claudePane.test.ts`;
untested main-process services; the experimental SQLite tables
(`pending_decisions`, `asks`, `events_fts`) left in place with the parked stack.

---

## 1. Desktop renderer (`apps/desktop/src/renderer`)

### Pane types
| Feature | Location | Status | Keep? |
|---|---|---|---|
| Terminal pane | `panes/TerminalPane.tsx`, `hooks/usePTY.ts` | WORKING | |
| Claude pane (terminal + GUI dual view) | `panes/ClaudePane.tsx`, `components/claude/*` | WORKING | |
| Browser pane (+ backs plugin panes) | `panes/BrowserPane.tsx` | WORKING | |
| Review / diff pane | `panes/ReviewPane.tsx`, `components/review/*`, `lib/gitQueries.ts` | WORKING | |
| Library pane (prompts/skills CRUD) | `panes/LibraryPane.tsx`, `hooks/useLibrary.ts` | WORKING | |
| Analytics pane | `panes/AnalyticsPane.tsx` | WORKING | |
| Overview pane (cross-agent stats) | `panes/OverviewPane.tsx` | WORKING | |
| Ask pane ("Ask the fleet" supervisor) | `panes/AskPane.tsx`, `askPresets.ts` | WORKING (`_sessionRefs` at line 116 is dead-in-live-file) | |
| Plugins Manager pane | `panes/PluginsManagerPane.tsx`, `hooks/usePlugins.ts` | WORKING | |
| Plugin pane (contributed webview) | `ScrollContainer.tsx:141` → BrowserPane | WORKING | |
| Settings pane (8 sections) | `panes/SettingsPane.tsx`, `components/settings/*` | WORKING | |
| **Notes pane** | `panes/NotesPane.tsx` | **HALF-BUILT** — renders "Notes placeholder - Phase 2"; reachable via palette (`new-notes`) | |
| **Agent pane** | `panes/AgentPane.tsx` | **HALF-BUILT** — renders "Agent placeholder - Phase 2"; reachable only via restore/layouts | |

> Note: no built-in **tracker / Jira / ADO / devops / daemon-dashboard** panes
> exist. They can only appear as plugin webview panes. If they were expected as
> first-class features, they are absent.

### Settings sections (all WORKING, persist via `saveConfig`)
Appearance (themes/corners), Layout (peek/gap), Keybindings (default/vim, remap,
leader, vim reference), Notifications, Session (auto-resume, Claude view pref),
Browser (hibernation timeout, Chrome cookie import), Apps (custom launcher CRUD),
Claude Profiles (CRUD).

### Cross-cutting UI surfaces
| Feature | Location | Status | Keep? |
|---|---|---|---|
| Command palette | `components/CommandPalette.tsx` | WORKING | |
| Keybindings (default/vim, chords, leader) | `hooks/useKeyboardNav.ts` | WORKING | |
| Shortcut overlay (help) | `components/ShortcutOverlay.tsx` | WORKING | |
| Sidebar (status dots, token/cost/context, counts) | `components/SideBar.tsx`, `HubStatus.tsx` | WORKING | |
| Jump-to-next-attention | `App.tsx:462` | WORKING | |
| **Triage Inbox (live)** | `components/InboxDrawer.tsx`, `contexts/AttentionContext.tsx`, `attention/*`, `lib/{attentionRouter,resolveAttention}.ts` | WORKING — fed by live snapshots (NOT the `/items` classifier) | |
| Fleet Deck (cross-agent radar) | `components/FleetDeck.tsx`, `AgentCard.tsx` | WORKING | |
| Session save/restore + picker | `hooks/useSessionLifecycle.ts`, `components/SessionPicker.tsx` | WORKING | |
| Layout sync (desktop⇄web mirror) | `hooks/useLayoutSync.ts` | WORKING | |
| Layout templates | `components/LayoutsDialog.tsx` | WORKING | |
| Browser hibernation (renderer logic) | `App.tsx:290-333`, BrowserPane | WORKING | |
| Spawn agent dialog | `components/SpawnAgentDialog.tsx` | WORKING | |
| Remote share dialog (QR) | `components/RemoteShareDialog.tsx` | WORKING | |
| Plugin install dialog | `components/PluginInstallDialog.tsx` | WORKING | |
| Library side panel / host / prompt-vars | `LibrarySidePanel.tsx`, `LibraryHost.tsx`, `PromptVarsDialog.tsx` | WORKING | |
| Bottom terminal panel | `components/BottomTerminalPanel.tsx` | WORKING | |
| NavBar (tabs, view-mode cycle, scripts) | `components/NavBar.tsx` | WORKING | |
| Spatial & stacked view modes | `ScrollContainer.tsx` | WORKING | |
| Themes (~18 variants) | `themes.ts`, `hooks/useTheme.ts` | WORKING | |

### DEAD CODE (renderer) — cleanup candidates
| File | Evidence | Keep? |
|---|---|---|
| `components/ItemDetailOverlay.tsx` | imported only by its own test | DROP? |
| `lib/claudemonItems.ts` | imported only by ItemDetailOverlay (dead) + test; sole `/items` client | DROP? |
| `lib/claudemonSessions.ts` | imported only by ItemDetailOverlay (dead) | DROP? |
| `components/claude/WorkingTimer.tsx` | zero importers | DROP? |
| `lib/terminalQueries.ts` | zero importers (live logic is in `terminalUtils.ts`) | DROP? |
| `components/ScrollIndicator.tsx` | zero importers | DROP? |
| `tests/components/ItemDetailOverlay.test.tsx` | tests a dead component | DROP? |

> `lib/claudemonBase.ts` is NOT dead — still used by live `gitQueries.ts`.
> `InboxPane` (mentioned in old docs) no longer exists; live equivalent is `InboxDrawer`.

---

## 2. Desktop main process (`apps/desktop/src/main`)

| Feature | Location | Status | Keep? |
|---|---|---|---|
| claudemon spawn/supervise | `services/claudemonDaemon.ts` | WORKING — **no auto-restart on crash** | |
| hub spawn/supervise | `services/hubDaemon.ts` | WORKING — **no auto-restart on crash** | |
| Binary resolution (dev + packaged) | `claudemonDaemon.ts:30`, `hubDaemon.ts:127`, `electron-builder.yml` | WORKING | |
| `claude` CLI resolver | `services/claudeResolver.ts` | WORKING (incl. Windows nvm fallback) | |
| IPC surface (44 channels) | `shared/ipcChannels.ts`, `ipc.ts`, `preload.ts` | WORKING (all wired) | |
| `claudeSessionStore` (3 SSE streams, hook routing, conversation deltas, usage) | `services/claudeSessionStore.ts`, `sessionStore/*` | WORKING — 38 tests | |
| OS notifications + taskbar flash | `services/agentNotifier.ts` | WORKING | |
| notifyDone / onlyWhenUnwatched / sound config | `agentNotifier.ts`, `configService.ts` | WORKING | |
| `WORKSPACER_DISABLE_GPU` escape hatch | `index.ts:140` | WORKING | |
| Session persistence (YAML save/restore) | `services/sessionService.ts` | WORKING | |
| SQLite analytics history | `services/sessionHistory.ts`, `db/database.ts` | WORKING | |
| Workflow watcher (subagent/workflow telemetry) | `services/workflowWatcher.ts` | WORKING | |
| Hub capabilities / MCP-facade provider (30+ methods) | `services/hubCapabilities.ts` | WORKING | |
| Remote sharing (`WORKSPACER_REMOTE_SHARE`) | `services/hubDaemon.ts` | WORKING (opt-in) | |
| Terminal share / remote PTY mirror | `services/terminalShare.ts` | WORKING | |
| Font discovery / Nerd Font serving | `index.ts` | WORKING | |
| Chrome UA spoofing for webviews | `index.ts` | WORKING | |
| Chrome cookie import (Windows CDP + direct) | `services/chromeCookieImport.ts` | WORKING (Windows only) | |
| Chrome cookie import (macOS/Linux) | `chromeCookieImport.ts:18` | **NOT BUILT** — throws on non-Windows | |
| Browser-pane hibernation backend | config only (`configService.ts`) | **HALF-BUILT** — enforcement is renderer-only | |
| Git/diff/merge backend | — | renderer→claudemon HTTP direct; **no merge endpoint**; `review_diff`/`merge` next-action wiring open | |
| `terminal:exit` IPC push | `ipcChannels.ts` + `preload.ts` | **DEAD** — declared + preload-subscribed, never sent | |
| Tray icon | — | **NOT BUILT** | |
| Overlay icon / dock badge | — | **NOT BUILT** (docs confirm) | |
| Crash recovery / auto-save journal | — | **NOT BUILT** — only `before-quit` signal | |
| E2E `claudePane.test.ts` | `tests/e2e/claudePane.test.ts` | **STALE** — POSTs to old port-7890 hook shape | |

> Unit tests: good for store/resolver/config/modelUsage/sse. Missing for
> agentNotifier, chromeCookieImport, daemon supervisors, hubClient,
> sessionService, workflowWatcher, terminalShare, sessionHistory.

---

## 3. claudemon — Rust daemon (`services/claudemon`)

| Feature | Location | Status | Keep? |
|---|---|---|---|
| Hook intake (`/hook`, `/hook/:kind`, `/statusline`) + deferred-hook gate | `daemon/hook.rs` | WORKING | |
| Session state machine (in-memory, DashMap) | `session/state.rs`, `session/store.rs` | WORKING | |
| Session/PTY APIs (input/output/stream/message/approve/answer/decide/gate/signal/resize/spawn) | `daemon/api.rs`, `daemon/spawn.rs` | WORKING | |
| Transcript read + conversation tailer (SSE deltas, seq-join) | `session/transcript.rs`, `session/conversation.rs` | WORKING | |
| Event/hook/statusline SSE streams (`/events`, `/hooks/stream`, `/statusline/stream`) | `daemon/api.rs` | WORKING | |
| Git API (status/diff/numstat/stage/unstage/commit/push) | `daemon/git.rs` | WORKING | |
| External wrapper + in-daemon PTY (WebSocket protocol) | `wrapper/*`, `protocol.rs` | WORKING | |
| `claudemon watch` TUI | `tui/*` | WORKING (cli.rs comment calls it a "stub" — misleading) | |
| `claudemon init` (merge hooks into settings.json) | `daemon/init.rs` | WORKING | |
| **Classifier (NeedsInput/Error/Stuck/Done + idle sweep)** | `classifier/*` | WORKING, 16 tests — **but output has no live consumer** | |
| **Inbox items API (`/items`, `/items/stream`, `/items/:id/action`)** | `daemon/api.rs:733-828`, `store/items.rs` | WORKING — **orphaned (only dead renderer code called it)** | |
| **SQLite items/events store** (`record_and_classify`, snooze wake) | `store/mod.rs`, `store/items.rs` | WORKING — backs the orphaned items API | |
| Summarizer (`/sessions/:id/summarize`, Haiku) | `daemon/api.rs:547` | WORKING (depends on `claude` on PATH + API access; failures silently → null) | |
| Signal delivery (SIGTERM/SIGKILL to child) | `daemon/spawn.rs`, `wrapper/mod.rs` | **HALF-BUILT** — only SIGINT (→Ctrl-C byte) delivered; SIGTERM/SIGKILL are `_ => {}` ("richer delivery TBD") | |
| Terminal emulator (vt100, answers device queries) | `session/emulator.rs` | **DEAD** — fully implemented + tested but **zero call sites**; viewers get raw PTY bytes, not emulated state | |
| SQLite `pending_decisions` table | `store/schema.rs` | **DEAD** — created, zero read/write code (v2-spec audit log, never built) | |
| SQLite `asks` table | `store/schema.rs` | **DEAD** — created, zero read/write code (v2-spec supervisor-ask log, never built) | |
| SQLite `events_fts` FTS5 table | `store/schema.rs` | **DEAD** — created, no population trigger, no `MATCH` queries (v2-spec transcript search, never wired) | |
| `ResolveAllForSession` item action | `classifier/types.rs:127` | **DEAD** — defined, never emitted | |
| `Db::path`, `OpenItem::priority`, `projects_dir()`, `encoded_cwd()` | various `#[allow(dead_code)]` | DEAD — minor | |

---

## 4. wks-tui — Rust terminal client (`apps/tui`)

| Feature | Location | Status | Keep? |
|---|---|---|---|
| Agent list / dashboard (SSE-driven) | `app/mod.rs`, `ui.rs` | WORKING | |
| Chat view — terminal path (vt100) | `terminal.rs`, `app/mod.rs` | WORKING | |
| Chat view — transcript path (syntax highlight) | `app/mod.rs`, `ui.rs` | WORKING | |
| Per-agent tabs (Claude + spawned shells) | `app/mod.rs` | WORKING | |
| Approve/answer/signal at list & chat level | `app/input.rs`, `claudemon.rs` | WORKING | |
| Spawn modal (cwd/profile/path-complete/library seed) | `app/mod.rs`, `app/tasks.rs` | WORKING | |
| Command palette (Ctrl-K, fuzzy) | `app/mod.rs` | WORKING | |
| Vim-style keyboard nav | `app/input.rs` | WORKING | |
| Daemon bootstrap (auto-spawn claudemon) | `daemons.rs`, `main.rs` | WORKING | |
| Test suite | — | **NONE** (only terminal key-encoding unit tests) | |

> wks-tui talks to claudemon HTTP directly; it does **not** use the hub bus and
> does **not** read `/items`.

---

## 5. hub — Go control-plane (`services/hub`)

| Feature | Location | Status | Keep? |
|---|---|---|---|
| In-memory pub/sub broker (drop-not-block) | `internal/broker` | WORKING (5 tests) | |
| WebSocket bus server `/bus` (64 MiB frames) | `internal/bus/bus.go` | WORKING | |
| RPC capability router (register/call/timeout) | `internal/bus/rpc.go` | WORKING (5 tests) | |
| Bus token auth (`--token`/Bearer/`?token=`) | `internal/bus/bus.go` | WORKING | |
| `/health` endpoint | `internal/bus/bus.go:118` | WORKING | |
| Reconnecting bus client | `internal/busclient` | WORKING (6 tests) | |
| Event envelope + topic matching | `internal/event` | WORKING (8 cases) | |
| Claudemon SSE→bus bridge | `internal/claudemon` | WORKING (integration-tested spine) | |
| Layout document service (get/set/persist/broadcast) | `internal/layout` | WORKING (4 tests) — **remote tmux-mirror, all 4 phases** | |
| `/remote` mobile web client + xterm mirror | `cmd/hub/remote.*` (embedded) | WORKING (client side; needs Electron providers) | |
| `/app/` full web app proxy | `cmd/hub/main.go:115` | WORKING (needs `--webapp-dir`) | |
| Plugin manifest + loader + installer (GitHub/tar.gz, zip-slip guard) | `internal/plugin` | WORKING (14 tests) | |
| Plugin manager + sidecar lifecycle | `internal/plugin/manager.go` | WORKING | |
| Process supervisor (health, restart, SIGTERM/KILL) | `internal/supervisor` | WORKING (3 tests) | |
| MCP facade (`/mcp` + `/sse`, 10 tools) | `cmd/mcp/main.go` | WORKING (2 integration tests; needs Electron providers) | |
| Example: rules-engine plugin | `examples/rules-engine` | WORKING (full engine, no tests) | |
| Example: rivet-bridge plugin | `examples/rivet-bridge` | WORKING (needs external `rivet` binary; one-project limit) | |
| Example: agent-dashboard plugin | `examples/agent-dashboard` | WORKING (hardcoded bus URL, no token) | |
| Example: clock-plugin | `examples/clock-plugin` | WORKING — but `emits: example.clock.tick` is **declared, never published** | |
| Per-method capability tokens (`SetAuthorize`) | `internal/bus/rpc.go:39`, `cmd/hub/main.go:58` | **HALF-BUILT** — seam wired, policy is allow-all (intentional) | |
| "Ask the fleet" supervisor agents | — | **NOT BUILT** in hub — only named in rivet-bridge comments; renderer AskPane is the real surface | |
| Auto-surface MCP-spawned sessions as panes | README "next milestones" | **NOT BUILT** | |
| `agent.snapshot` lightweight event | `docs/rules-engine-plugin.md §12` | **NOT BUILT** (deferred) | |

---

## 6. Cross-cutting decisions to make

These are the genuine keep/drop / finish/cut calls (not obvious cleanups):

1. **Orphaned classifier + `/items` + SQLite inbox stack (claudemon).** Three
   options: (a) **revive** — wire it to a renderer surface (replacing or feeding
   the live InboxDrawer); (b) **drop** — delete classifier/items/SQLite-items +
   the dead renderer client chain, shrinking claudemon significantly; (c) **park**
   — keep building, tag it clearly experimental, ship without it.

2. **Two inbox systems.** If we keep the live InboxDrawer (option for #1 = drop),
   confirm that's the canonical triage surface and remove the v2-spec inbox
   ambition from docs.

3. **`docs/v2-spec.md`.** Largely superseded. Keep as historical record, rewrite
   to match the shipped product, or delete.

4. **Phase-2 placeholder panes (Notes, Agent).** Finish, or remove (and drop the
   `new-notes` palette entry).

5. **Daemon supervision & signals.** No auto-restart on crash for claudemon/hub
   (recommend: add supervised restart). Separately, claudemon only delivers
   SIGINT to child sessions — SIGTERM/SIGKILL are stubbed, so a runaway Claude
   session can't be cleanly killed. Both are likely production blockers.

6. **Platform gaps.** macOS/Linux Chrome cookie import (NOT BUILT); tray icon /
   overlay badge (NOT BUILT). Keep Windows-only, or implement, or cut cookie
   import entirely?

7. **Git review loop.** Read/stage/commit/push work; `merge` + `review_diff`
   next-action wiring is open. Finish the loop or leave manual?

8. **Stale/missing tests.** E2E `claudePane.test.ts` is architecturally stale;
   many main-process services untested. Fix before calling it production-ready.

9. **Example plugins.** Keep all four as shipped examples, or trim
   (clock-plugin's emit is fake; rivet-bridge needs an external binary)?
