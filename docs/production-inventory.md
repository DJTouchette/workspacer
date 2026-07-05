# Workspacer — Production-Readiness Feature Inventory

> Compiled 2026-06-13 from a full read of all four components after the
> `apps/` + `services/` monorepo refactor.
>
> **Refreshed 2026-07-05:** reconciled the doc against the code. Key deltas since
> the original compile: the classifier + `/items` + SQLite-items inbox stack was
> **removed** from claudemon (July 2026); daemon auto-restart and real
> SIGTERM/SIGKILL delivery are now **built**; per-plugin capability grants are now
> **enforced** (verb + path scoping); the MCP facade has grown to ~45 tools; and
> several new subsystems landed (headless `brain` capability provider, `/m` mobile
> client, cross-provider handoff, live permission-mode/model switching, the
> `agent.snapshot` / `agent.statusline` / `pty.bytes` bus topics). Details inline.
>
> Status legend:
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

2. **RESOLVED (removed July 2026): the orphaned classifier + `/items` inbox.**
   The original compile flagged an entire orphaned backend subsystem — a
   classifier (`NeedsInput / Error / Stuck / Done`) writing priority-ranked items
   to SQLite and serving them over `/items`, `/items/stream`,
   `/items/:id/action`, with no live consumer. That whole stack has since been
   **deleted** from claudemon: `src/classifier/` is gone, the `/items` routes are
   gone, and `store/schema.rs` now creates only `sessions` + `events` (the
   v2-spec `pending_decisions`, `asks`, and `events_fts` tables are gone too). The
   dead renderer client chain (`claudemonItems.ts` → `ItemDetailOverlay.tsx`) was
   removed alongside it. The claudemon `/sessions/:id/summarize` (Haiku) endpoint
   was also removed in this cull. The SQLite store is now purely session/event
   cold-storage for resume-after-restart.

3. **One triage system, not two.** With the classifier pipeline removed, the
   renderer's live triage surface is the only one: `InboxDrawer` +
   `AttentionContext`, fed from live `ClaudeSessionSnapshot` ambient state. The
   "two inbox systems" duplication called out in the original compile no longer
   exists.

Everything else is mostly healthy. Remaining decisions concentrate in: a couple
of dead renderer files, two "Phase 2" placeholder panes, and platform gaps
(macOS/Linux cookie import, tray/overlay icons). Daemon auto-restart and
SIGTERM/SIGKILL delivery — flagged as gaps in the original compile — are now
built (see §2/§3).

---

## Decisions taken (2026-06-13)

| Decision | Outcome | Status |
|---|---|---|
| Classifier + `/items` + SQLite inbox | Parked as experimental on 2026-06-13; **later REMOVED entirely (July 2026)** — `classifier/`, `/items` routes, and the SQLite items/`pending_decisions`/`asks`/`events_fts` tables all deleted | ✅ removed |
| Dead renderer client chain (`claudemonItems`, `claudemonSessions`, `ItemDetailOverlay`) | Deleted (dead regardless of the park) | ✅ done |
| Other dead renderer files (`WorkingTimer`, `terminalQueries`, `ScrollIndicator`) | Deleted | ✅ done |
| NotesPane | **Finished** — real per-agent markdown scratchpad, persists with the session | ✅ done |
| AgentPane placeholder + `'agent'` pane type | Removed | ✅ done |
| `docs/v2-spec.md` | Deleted (superseded) | ✅ done |
| claudemon/hub auto-restart on crash | Implemented (exponential backoff supervision) | ✅ done |
| SIGTERM/SIGKILL delivery to sessions | Implemented (`pty::signal_child`, real POSIX signals) | ✅ done |
| Pre-existing renderer typecheck errors (KeybindingsConfig, createTerminal decl) | Fixed in passing | ✅ done |

**Still open (as of the 2026-07-05 refresh):** macOS/Linux Chrome cookie import;
tray/overlay icon; git merge / `review_diff` next-action wiring; untested
main-process services. (The stale E2E `claudePane.test.ts` was retired — see
below.) (The experimental SQLite
tables `pending_decisions` / `asks` / `events_fts` that were left in place with
the parked stack have since been removed — see the classifier cull in §0 and §3.)

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
| Plugin pane (contributed webview) | `ScrollContainer.tsx` `case 'plugin'` → BrowserPane | WORKING | |
| Editor pane (`'editor'` type) | `ScrollContainer.tsx` `case 'editor'` | WORKING — terminal engine runs the user's `$EDITOR`/nvim in a PTY (`config.editor.terminalCommand`); the removed in-app CodeMirror editor is now the sandboxed `workspacer.editor` plugin, opened via "Open Editor" | |
| Settings pane (Editor section added) | `panes/SettingsPane.tsx`, `components/settings/*` | WORKING | |
| Notes pane | `panes/NotesPane.tsx` | WORKING — real per-agent markdown scratchpad (finished per Decisions 2026-06-13); reachable via palette (`new-notes`) | |
| ~~Agent pane~~ | — | REMOVED (placeholder + `'agent'` pane type deleted per Decisions 2026-06-13) | |

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

### DEAD CODE (renderer) — RESOLVED
The dead renderer files flagged in the original compile have all been deleted:
`ItemDetailOverlay.tsx`, `lib/claudemonItems.ts`, `lib/claudemonSessions.ts`
(the dead `/items` client chain), plus `components/claude/WorkingTimer.tsx`,
`lib/terminalQueries.ts`, `components/ScrollIndicator.tsx`, and the
`ItemDetailOverlay.test.tsx` test. None remain in the tree.

> `InboxPane` (mentioned in old docs) no longer exists; live equivalent is `InboxDrawer`.

---

## 2. Desktop main process (`apps/desktop/src/main`)

| Feature | Location | Status | Keep? |
|---|---|---|---|
| claudemon spawn/supervise | `services/claudemonDaemon.ts` | WORKING — **auto-restart on crash** via `RestartBackoff` (exponential backoff, gives up after repeated failures with a user-facing warning) | |
| hub spawn/supervise | `services/hubDaemon.ts` | WORKING — auto-restart on crash (shared `RestartBackoff` from `lib/daemonUtils`) | |
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
| E2E `claudePane.test.ts` | (removed) | **RETIRED** — premise obsolete: 7890 is now claudemon's hook ingress (not the desktop's), and the hook→GUI path round-trips through a claudemon the test never starts. A runnable replacement would need claudemon + a native folder-dialog stub. Its intended coverage now lives in renderer component tests (`tests/components/ClaudePane.test.tsx` send/approval/question seams, `composerControls`/`composer`/`needsYou`) + claudemon's own hook tests + `claudemonHookBridge.test.ts`. | |

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
| Git (review pane) — MOVED to the desktop host (`apps/desktop/src/main/services/gitService.ts`, exposed as `git:*` IPC + `git.*` hub capabilities). Was `daemon/git.rs`. | — | MOVED OUT OF CLAUDEMON | |
| External wrapper + in-daemon PTY (WebSocket protocol) | `wrapper/*`, `protocol.rs` | WORKING | |
| `claudemon watch` TUI | `tui/*` | WORKING (cli.rs comment calls it a "stub" — misleading) | |
| `claudemon init` (merge hooks into settings.json) | `daemon/init.rs` | WORKING | |
| Cross-provider handoff (`POST /sessions/:id/handoff`) | `daemon/api.rs`, `session/handoff.rs` | WORKING — builds a deterministic brief, persists to `~/.workspacer/handoffs/` unless `no_persist`; successor spawns with the composer pre-filled to read it | |
| Live permission-mode switch (`POST /sessions/:id/permission-mode`) | `daemon/api.rs` | WORKING — no restart, conversation untouched (claude via shift+tab screen, adapters via provider flag) | |
| Live model switch (`POST /sessions/:id/model`, `GET /providers/:provider/models`) | `daemon/api.rs`, `daemon/spawn.rs` | WORKING for adapter providers; PTY (claude) sessions switch via the `/model` slash command on the message path | |
| PTY-bytes SSE (`pty.bytes` events) | `daemon/api.rs` | WORKING — bridged to the hub bus, consumed by the headless `brain` + web/TUI/mobile clients | |
| ~~Classifier (NeedsInput/Error/Stuck/Done + idle sweep)~~ | — | **REMOVED (July 2026)** — `src/classifier/` deleted; live triage is the renderer `InboxDrawer` fed by snapshots | |
| ~~Inbox items API (`/items`, `/items/stream`, `/items/:id/action`)~~ | — | **REMOVED (July 2026)** — routes and `store/items.rs` deleted | |
| SQLite session/event cold-storage | `store/mod.rs`, `store/schema.rs` | WORKING — now only `sessions` + `events` tables; `load_recent_sessions` rehydrates the in-memory list on boot so prior agents reappear as resumable | |
| ~~Summarizer (`/sessions/:id/summarize`, Haiku)~~ | — | **REMOVED (July 2026)** with the classifier cull | |
| Signal delivery (SIGTERM/SIGKILL to child) | `daemon/spawn.rs`, `wrapper/mod.rs` | WORKING — non-SIGINT signals routed via `pty::signal_child` (real POSIX signals); SIGINT still delivered as a Ctrl-C byte; shutdown sends SIGKILL | |
| ~~Terminal emulator (vt100)~~ | — | **REMOVED (July 2026)** — `session/emulator.rs` deleted; viewers get raw PTY bytes (emulation lives client-side) | |
| ~~SQLite `pending_decisions` / `asks` / `events_fts` tables~~ | — | **REMOVED (July 2026)** — v2-spec tables deleted from `store/schema.rs` (only `sessions` + `events` remain) | |

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

> wks-tui now defaults to the **hub bus** (auto-spawning the hub + headless
> `brain`); pass `--direct` for the standalone claudemon-HTTP path. The `/items`
> API it never used has since been removed.

---

## 5. hub — Go control-plane (`services/hub`)

> **Security:** for the remote-sharing threat model (loopback default, tailnet
> deployment, token-in-URL, sandbox caveats), see
> [`docs/remote-sharing-security.md`](remote-sharing-security.md).

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
| `/remote` web client + xterm mirror | `cmd/hub/remote.*` (embedded) | WORKING (client side; needs a capability provider) | |
| `/m` mobile-first remote client | `cmd/hub/main.go` `/m` route, `cmd/hub/mobile.html` (embedded) | WORKING (client side) | |
| Headless `brain` capability provider | `cmd/brain/*` | WORKING — provides the agent view + capabilities off the bus so web/TUI/mobile clients work without the desktop app; `--scope full` (headless, owns the session store) or `--scope catalog` (file-backed subset, runs alongside the desktop app); env `WKS_BRAIN_SCOPE` | |
| Bus topics `agent.snapshot` / `agent.statusline` / `pty.bytes` | `cmd/brain/store.go`, `internal/claudemon` bridge | WORKING — the lightweight agent-list + statusline + live-terminal feeds all clients subscribe to | |
| `/app/` full web app proxy | `cmd/hub/main.go:115` | WORKING (needs `--webapp-dir`) | |
| Plugin manifest + loader + installer (GitHub/tar.gz, zip-slip guard) | `internal/plugin` | WORKING (14 tests) | |
| Plugin manager + sidecar lifecycle | `internal/plugin/manager.go` | WORKING | |
| Process supervisor (health, restart, SIGTERM/KILL) | `internal/supervisor` | WORKING (3 tests) | |
| MCP facade (`/mcp` + `/sse`, ~45 tools) | `cmd/mcp/main.go` | WORKING (2 integration tests; needs a capability provider) — a thin adapter forwarding each tool call to a hub bus method (list/spawn/drive agents, terminals, fs, config, profiles, sessions, layouts, library, analytics, notify) | |
| Example: rules-engine plugin | `examples/rules-engine` | WORKING (full engine, no tests) | |
| Example: rivet-bridge plugin | `examples/rivet-bridge` | WORKING (needs external `rivet` binary; one-project limit) | |
| Example: agent-dashboard plugin | `examples/agent-dashboard` | WORKING (hardcoded bus URL, no token) | |
| Example: clock-plugin | `examples/clock-plugin` | WORKING — but `emits: example.clock.tick` is **declared, never published** | |
| Per-plugin capability grants (verb + path scoping) | `internal/bus/rpc.go` (`authorize`), `internal/capspec`, `internal/bus/policy.go` | WORKING — a plugin is authorized only for the capability methods (verbs) its grant declares, and path-scoped methods (`fs.*`, `search.project`) are confined to the grant's canonical, symlink-resolved roots; unauthorized calls are denied ("plugin not authorized for capability") | |
| "Ask the fleet" supervisor agents | — | **NOT BUILT** in hub — only named in rivet-bridge comments; renderer AskPane is the real surface | |
| Auto-surface MCP-spawned sessions as panes | README "next milestones" | **NOT BUILT** | |
| `agent.snapshot` lightweight event | `cmd/brain/store.go` | WORKING — published to the bus by the `brain`; foundation of the agent-list feed (see the bus-topics row above) | |

---

## 6. Cross-cutting decisions to make

These are the genuine keep/drop / finish/cut calls (not obvious cleanups):

1. ~~**Orphaned classifier + `/items` + SQLite inbox stack (claudemon).**~~
   **RESOLVED — dropped (July 2026).** The classifier, `/items` routes, SQLite
   items store, and the dead renderer client chain were all deleted; claudemon's
   SQLite is now session/event cold-storage only. The live `InboxDrawer` is the
   canonical triage surface.

2. ~~**Two inbox systems.**~~ **RESOLVED** — only the live `InboxDrawer` remains
   (see #1).

3. **`docs/v2-spec.md`.** Deleted per Decisions 2026-06-13 (superseded).

4. **Phase-2 placeholder panes.** Partially resolved: NotesPane was finished and
   the AgentPane placeholder + `'agent'` type were removed (Decisions 2026-06-13).

5. ~~**Daemon supervision & signals.**~~ **RESOLVED.** claudemon/hub now
   auto-restart on crash with exponential backoff (`RestartBackoff`), and
   claudemon delivers real SIGTERM/SIGKILL to child sessions via
   `pty::signal_child` (a runaway session can be cleanly killed).

6. **Platform gaps.** macOS/Linux Chrome cookie import (NOT BUILT); tray icon /
   overlay badge (NOT BUILT). Keep Windows-only, or implement, or cut cookie
   import entirely?

7. **Git review loop.** Read/stage/commit/push work; `merge` + `review_diff`
   next-action wiring is open. Finish the loop or leave manual?

8. **Stale/missing tests.** E2E `claudePane.test.ts` was architecturally stale
   and has been retired (see the capabilities table); its coverage moved to
   renderer component tests. Many main-process services remain untested — address
   before calling it production-ready.

9. **Example plugins.** Keep all four as shipped examples, or trim
   (clock-plugin's emit is fake; rivet-bridge needs an external binary)?
