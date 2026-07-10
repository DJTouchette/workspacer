# Workspacer — Feature Assessment

> Current-state catalog of what the app does, as of 2026-07-09. For the
> keep/drop triage and dead-code notes, see `docs/production-inventory.md`.

**Maturity legend**

- 🟢 **Solid** — complete, wired end-to-end, has test coverage.
- 🔵 **Working** — complete and wired; lighter or no automated tests.
- 🟡 **Partial** — usable but incomplete (gaps noted).
- 🟣 **Experimental / Parked** — built but not surfaced; kept as future substrate.
- ⚪ **Not built** — referenced/configured but no implementation.

## What Workspacer is

A local-first cockpit for running **many long-lived coding agents side by
side** — Claude Code, Codex, OpenCode, and Pi (beta). An Electron + React
desktop app is the primary client; a Rust daemon (`claudemon`) owns the
sessions/PTYs and runs the per-provider managed adapters; a Go control-plane
(`hub`) is an event bus + MCP facade that lets plugins, a Rust TUI, and
remote/web/phone clients all view and drive the same fleet.

---

## 1. Multi-agent workspace (core model)

| Feature | What it does | Maturity |
|---|---|---|
| Agent workspaces | Each agent = one long-lived claudemon session (by cwd) with its own tabs/panes; lives in the daemon independent of the UI | 🟢 Solid |
| Spawn agent | Pick cwd / backend (Claude, Codex, OpenCode, Pi) / model / profile / permission mode / transport; seed an initial prompt | 🔵 Working |
| Auto-adopt | Sessions spawned externally (MCP, another agent) appear as cards automatically | 🔵 Working |
| Supervisor agents | Spawn a Claude with the workspacer MCP facade attached; rendered nested under its parent | 🔵 Working |
| Respawn / terminate | Restart a stopped agent (re-points its Claude panes); kill an agent's session | 🔵 Working |
| Rename / reorder agents | Per-agent display name, sidebar ordering | 🔵 Working |
| Deterministic agent identity | Cards keyed by a stable id derived from the session, deduped on layout sync — prevents cross-client duplication | 🟢 Solid |

## 2. Agent interaction

| Feature | What it does | Maturity |
|---|---|---|
| Agent pane — terminal mode | Live PTY view of a terminal-transport Claude session | 🔵 Working |
| Agent pane — GUI mode | Rich conversation view: approve/deny, answer questions, inline diffs, work log, per-turn changed-files cards | 🔵 Working |
| Claude stream transport | Headless `--print` stream-json adapter (`claude_stream.rs`) behind `claude.transport` (shipped default `stream`); control-protocol approvals/questions/model/mode | 🔵 Working |
| Managed providers | Codex (`codex app-server`), OpenCode (`opencode serve` + SSE), Pi (`pi --mode rpc`, beta) driven natively by claudemon adapters | 🔵 Working |
| Inspector rail | Files / Plan / Workflows / Subagents / Usage (5h / 7d / monthly rate windows) for the active session | 🔵 Working |
| Composer | Send messages with file attach (drag / paste / picker); streaming + cancel; model / effort / permission-mode pills | 🔵 Working |
| Live model & mode switching | Switch model and permission mode mid-session without a respawn (PTY-verified or control-protocol per transport) | 🔵 Working |
| Approvals & questions | One-key approve/deny, AskUserQuestion answering with multi-select and "Decline & stop", persistent answered cards | 🔵 Working |
| Cross-provider handoff | `POST /sessions/:id/handoff` builds a brief in `~/.workspacer/handoffs/`; successor spawns pre-pointed at it | 🔵 Working |
| Subagent & workflow telemetry | Live phases, per-agent tokens/tools surfaced from the daemon's workflow watcher; dedicated watch panes | 🔵 Working |
| Claude profiles | Named profiles (model, config dir, extra args, MCP items) selectable at spawn | 🔵 Working |

## 3. Pane types

| Feature | What it does | Maturity |
|---|---|---|
| Terminal | xterm + PTY shell pane | 🔵 Working |
| Browser | Embedded webview with nav/bookmarks/app-mode + theme injection; backs plugin panes | 🔵 Working |
| Review / diff | Git status + unified diff viewer with file tree; opened from a Claude pane | 🔵 Working |
| Notes | **Per-agent markdown scratchpad** (write/split/preview) that persists with the session | 🔵 Working |
| Library | CRUD for reusable prompts / skills / agents | 🔵 Working |
| Analytics | Cost/token totals, by-project/by-model, recent sessions | 🔵 Working |
| Overview | Cross-agent stats + rate-limit card + recent/favourite dirs to spawn | 🔵 Working |
| Ask ("Ask the fleet") | Spawn a supervisor from a question, with preset chips | 🔵 Working |
| Editor | Sandboxed CodeMirror plugin (default) or `$EDITOR` in a PTY (`editor.engine`) | 🔵 Working |
| Agents / agent watch | Fleet monitor of a session's subagents; click-through watch panes for live subagents & workflow runs; pinnable inspector pane | 🔵 Working |
| Markdown preview | Read-only rendered markdown (`mdpreview`), opened from file links in chat | 🔵 Working |
| Plugins Manager | List/install/remove plugins with sidecar health | 🔵 Working |
| Plugin pane | Generic webview host for a plugin's own UI (this is how any tracker/devops/dashboard surface appears) | 🔵 Working |
| Settings | 13 sections (incl. theme maker, updates), all persisted | 🔵 Working |
| Bottom terminal panel | Toggleable drawer terminal | 🔵 Working |

## 4. Navigation & layout

| Feature | What it does | Maturity |
|---|---|---|
| Command palette | Fuzzy launcher for actions, apps, plugins, library | 🔵 Working |
| Keybindings | Direct combos + prefix chords (leader), remap capture, shortcut overlay, which-key hints | 🔵 Working |
| UI modes | `fleet` (full cockpit) / `focus` (chat-first rail) lenses via `config.ui.mode` — without remounting live panes | 🔵 Working |
| Fleet Deck | Cross-agent "radar" of live agent cards, a global altitude orthogonal to UI mode | 🔵 Working |
| Layout templates | Save/restore named agent+pane arrangements | 🔵 Working |
| Session save / restore | Auto-resume, crash-recovery picker, legacy migration | 🔵 Working |
| Themes | 18 built-in themes via `--wks-*` CSS tokens, plus a theme maker (`ui.customThemes`) | 🔵 Working |
| Custom app launcher | User-defined external app/site shortcuts | 🔵 Working |

## 5. Attention & notifications

| Feature | What it does | Maturity |
|---|---|---|
| Sidebar status | Per-agent ambient state dot, context %, token/cost; "N need you / N working" header | 🔵 Working |
| Triage Inbox | Top-level drawer of attention items (approve/answer/dismiss/snooze) fed by live session snapshots | 🔵 Working |
| Jump-to-next-attention | Hotkey to cycle to the next agent waiting on you | 🔵 Working |
| OS notifications | Fire on needs-approval / needs-input and (optionally) done; suppressed for the watched agent; taskbar flash | 🔵 Working |
| Notification config | `enabled` / `notifyDone` / `onlyWhenUnwatched` / `sound` | 🔵 Working |
| Mobile push | Background Web Push to the `/m` PWA on needs-approval / needs-input (hub `internal/push`, VAPID) | 🔵 Working |
| Per-session budgets | Cost ceiling per session with a one-shot notification when crossed | 🔵 Working |
| Tray / overlay badge | — | ⚪ Not built |

## 6. Git / review

| Feature | What it does | Maturity |
|---|---|---|
| Git status / diff / numstat | Read paths, served by claudemon, rendered in the Review pane | 🔵 Working |
| Stage / unstage / commit / push | Mutating git ops from the Review pane | 🔵 Working |
| Merge / `review_diff` next-action wiring | Classifier emits these as next-actions; no UI behind them | ⚪ Not built |

## 7. Library, analytics & profiles

| Feature | What it does | Maturity |
|---|---|---|
| Library (prompts/skills/agents) | Create, edit, scope, and fire reusable items into sessions; variable templating | 🔵 Working |
| Quick-pick panel | Ctrl+L library picker | 🔵 Working |
| Analytics history | SQLite per-session history (model, cost, tokens, tools, branch); summary + recent | 🔵 Working |

## 8. Remote & multi-client

| Feature | What it does | Maturity |
|---|---|---|
| Hub event bus | WebSocket pub/sub + RPC capability router; bus token auth | 🟢 Solid |
| Layout mirroring (tmux-style) | Hub-owned shared layout doc; desktop ⇄ web/phone mirror cards, tabs, active tab | 🔵 Working |
| Mobile PWA (`/m`) | Phone-first decision client (fleet, Needs You queue, chat, spawn); installable PWA with background Web Push, the default QR target | 🔵 Working |
| Terminal-mirror client (`/remote`) | Lightweight client: agent list, chat, approvals, live xterm terminal mirror | 🔵 Working |
| Full web app (`/app/`) | Serves the real renderer bundle over the bus (when remote sharing + web build present) | 🔵 Working |
| Remote sharing | Runtime toggle (Start sharing) or `WORKSPACER_REMOTE_SHARE`; binds off-loopback with a bearer token; QR share dialog | 🔵 Working |
| Tailscale HTTPS | One-tap `tailscale serve` fronting the hub at `https://<node>.ts.net` (secure context for the PWA + push) | 🔵 Working |
| Terminal share / PTY mirror | Lease-gated PTY byte streaming to remote viewers | 🔵 Working |
| Cross-client de-duplication | One card per session across all clients (deterministic ids + dedupe) | 🟢 Solid |

## 9. Plugins & extensibility (hub)

| Feature | What it does | Maturity |
|---|---|---|
| Plugin manifest + loader | `plugin.json` schema (server, panes, hotkeys, capabilities); validated dir scan | 🟢 Solid |
| Plugin manager | Start/stop sidecars, lifecycle events, atomic install/remove | 🟢 Solid |
| Plugin installer | Install from GitHub URL / tarball, zip-slip-guarded, runs build step | 🟢 Solid |
| Process supervisor | Health-polled sidecars with SIGTERM→SIGKILL stop and restart backoff | 🟢 Solid |
| Plugin theming | `--wks-*` CSS token injection into plugin webviews | 🔵 Working |
| Example: rules-engine | Event→action interpreter (notify/sendMessage/command/emit/webhook) with a web editor | 🔵 Working |
| Example: rivet-bridge | Bridges `rivet` MCP tools onto the bus (recon/witness/schema); needs external `rivet` binary | 🔵 Working |
| Example: agent-dashboard | Bus-native agent grid webview | 🔵 Working |
| Example: clock-plugin | Minimal demo webview | 🔵 Working |

## 10. MCP facade ("Ask the fleet" backend)

| Feature | What it does | Maturity |
|---|---|---|
| MCP server | `/mcp` (Streamable HTTP) + `/sse`, exposing the fleet to ephemeral `claude -p` supervisors | 🟢 Solid |
| MCP tools (~38) | The driving set (list_agents, get_transcript, spawn_agent, create_terminal, send_message, approve, answer, signal, terminal_input, notify) plus snapshots/conversations, config/profiles, saved sessions/layouts, library, analytics, and path-scoped fs/search | 🟢 Solid |
| Per-method capability tokens | Authorization seam wired allow-all; real policy not implemented | 🟡 Partial |

## 11. Terminal client — `wks-tui` (Rust)

| Feature | What it does | Maturity |
|---|---|---|
| Dashboard + chat | Agent list, transcript & live-terminal views, per-agent tabs | 🔵 Working |
| Spawn / approve / answer / signal | Full agent control from the terminal | 🔵 Working |
| Command palette, vim nav | Ctrl-K fuzzy palette; leader + which-key, Ctrl-w splits, harpoon pins, `/` filter, `:` cmdline, vim counts | 🔵 Working |
| Daemon bootstrap | Auto-spawns claudemon if not running | 🔵 Working |
| Tests | Only terminal key-encoding units; no suite | 🟡 Partial |

## 12. Substrate — `claudemon` daemon (Rust)

| Feature | What it does | Maturity |
|---|---|---|
| Hook intake | Receives all Claude Code hook events; deferred-hook approval gate | 🟢 Solid |
| Managed provider adapters | `claude_stream` / `codex` / `opencode` / `pi`: pure per-provider `translate()` + shared `apply_updates`, spawned via `POST /sessions/spawn-managed` | 🟢 Solid |
| Session state machine | In-memory per-session mode/state, broadcast over SSE | 🟢 Solid |
| Session/PTY APIs | input/output/stream/message/approve/answer/decide/gate/resize/spawn | 🟢 Solid |
| Transcript + conversation tailer | Parse JSONL, stream deltas with sequence-join | 🟢 Solid |
| Git API | status/diff/numstat/stage/unstage/commit/push | 🔵 Working |
| Signal delivery | SIGINT (Ctrl-C), **and real SIGTERM/SIGKILL** to sessions | 🟢 Solid |
| Wrapper protocol | External-process PTY mirroring over WebSocket | 🔵 Working |
| `claudemon watch` TUI | Built-in dashboard + chat TUI | 🔵 Working |
| `claudemon init` | Merge hooks into `~/.claude/settings.json` | 🔵 Working |
| Summarizer | One-shot Haiku summary endpoint | 🔵 Working |

## 13. Platform, packaging & ops

| Feature | What it does | Maturity |
|---|---|---|
| Daemon supervision | Electron spawns claudemon + hub; **auto-restart on crash with backoff** | 🟢 Solid |
| Binary resolution + packaging | Dev/packaged paths; electron-builder bundles daemons | 🔵 Working |
| GPU escape hatch | `WORKSPACER_DISABLE_GPU` for broken Wayland GPU rendering | 🔵 Working |
| Font discovery | Nerd Font scan + custom protocol serving | 🔵 Working |
| Chrome UA spoofing | Strips Electron UA for webviews | 🔵 Working |
| Chrome cookie import | Windows (CDP + direct); macOS/Linux not implemented | 🟡 Partial |
| Toolchains | Pinned via mise (Go 1.25, Node 22) + cargo | 🔵 Working |

---

## Parked / experimental (built, not surfaced)

| Feature | Notes | Maturity |
|---|---|---|
| Classifier (Stuck/Error/Done + idle) | Detects per-session states; fully tested | 🟣 Parked |
| `/items` inbox API + SQLite items store | Priority-ranked inbox; no live consumer (the shipped UI uses the snapshot-based Triage Inbox instead) | 🟣 Parked |
| SQLite `pending_decisions` / `asks` / `events_fts` | v2-spec tables created but unused | 🟣 Parked |
| Terminal emulator (`emulator.rs`) | vt100 emulator with device-query replies; not wired into the output path | 🟣 Parked |

## Known gaps / not built

- Tray icon / taskbar overlay badge.
- Git **merge** and `review_diff`/`merge` next-action UI wiring.
- macOS/Linux Chrome cookie import.
- Per-method capability tokens (authorization seam is allow-all).
- Crash-recovery journal (only a `before-quit` save signal today).
- Browser-pane hibernation is renderer-only (no main-process enforcement).
- Stale E2E `claudePane.test.ts`; several main-process services untested.

---

## Overall assessment

The **substrate is strong**: the Go hub (bus, RPC, plugins, MCP facade,
supervisor) and the Rust claudemon (hooks, sessions, PTY, transcripts, git) are
complete and well-tested. The **desktop app is feature-rich and cohesive** —
~30 working UI surfaces over a fully-wired main process with solid session-store
test coverage. Recent hardening closed the biggest production risks: daemon
auto-restart, real terminate/kill signals, and the cross-client agent-duplication
fix.

The main *product* ambiguity is resolved: the app is a **per-agent workspace**,
not the abandoned v2 "inbox of decisions" — the leftover classifier/items stack
is now clearly parked. Remaining work is breadth (platform parity, a few
unfinished loops like git merge) and test depth (main-process services, an
end-to-end multi-client check), not core soundness.
