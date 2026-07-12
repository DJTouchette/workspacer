# Workspacer — Feature Assessment

> Current-state catalog of what the app does, as of 2026-07-12.

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
| Managed providers | Codex (`codex app-server`), OpenCode (`opencode serve` + SSE), Pi (`pi --mode rpc`, beta) driven natively by claudemon adapters; all get GUI approval/question cards, structural turn-interrupt, and live token/cost | 🔵 Working |
| Codex stream transport | Spawn dialog offers `hybrid` (native Codex TUI + GUI, one thread) or `headless` (`transport:"stream"` — daemon-owned `thread/start`, GUI-only); restart preserves the transport | 🔵 Working |
| Inspector rail | Files / Plan / Workflows / Subagents / Usage (5h / 7d / monthly rate windows) for the active session | 🔵 Working |
| Composer | Send messages with file attach (drag / paste / picker); streaming + cancel; model / effort / permission-mode pills | 🔵 Working |
| Live model & mode switching | Switch model and permission mode mid-session without a respawn (PTY-verified or control-protocol per transport) | 🔵 Working |
| Approvals & questions | One-key approve/deny, AskUserQuestion answering with multi-select and "Decline & stop", persistent answered cards — on every provider (managed providers get `AskUserQuestion` via claudemon's per-session MCP shim) | 🔵 Working |
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
| Settings | 14 sections (incl. theme maker, updates, Command Line install), all persisted | 🔵 Working |
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
| Mobile push | Background Web Push to the `/m` PWA on needs-approval / needs-input (hub `internal/push`, VAPID, needs HTTPS); now also works under a pure headless `workspacer serve` (brain snapshots carry ambient state), but delivery is not yet reliable | 🟡 Partial |
| Per-session budgets | Cost ceiling per session with a one-shot notification when crossed | 🔵 Working |
| Tray / overlay badge | — | ⚪ Not built |

## 6. Git / review

| Feature | What it does | Maturity |
|---|---|---|
| Git status / diff / numstat | Read paths, served by claudemon, rendered in the Review pane | 🔵 Working |
| Stage / unstage / commit / push | Mutating git ops from the Review pane | 🔵 Working |
| Merge / `review_diff` next-action wiring | Review next-actions route into the Review pane; merge/conflict/no-upstream/push-rejection states surface as actionable Review errors | 🔵 Working |

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
| Mobile PWA (`/m`) | Phone-first decision client, redesigned 2026-07: Needs You queue, full fleet list (needs-you → working → idle → stopped, same visibility rule as the desktop sidebar), per-agent chat with history (fetched over the bus under headless serve), spawn; installable PWA, the default QR target (push: see §5) | 🔵 Working |
| Terminal-mirror client (`/remote`) | Lightweight client: agent list, chat, approvals, live xterm terminal mirror | 🔵 Working |
| Full web app (`/app/`) | Serves the real renderer bundle over the bus (desktop: when sharing is on + web build present; `workspacer serve` auto-serves a sibling `web/` — bundled in the server archive) | 🔵 Working |
| Headless server (`workspacer serve`) | One command starts + supervises claudemon and the hub with a full-scope brain provider (restart backoff, parentwatch); prints the /m + /remote + /app URLs and pairing token (`--json` for machines); loopback by default, `--host` is the remote opt-in; `workspacer status` probes all three | 🔵 Working |
| Headless spawn parity | The brain's `agents.spawn` dispatches every backend like the desktop does (managed providers via spawn-managed, codex stream transport, resume), with a drift guard against the desktop's param surface | 🔵 Working |
| Desktop app as a client | Adopt-don't-kill: the app attaches to a healthy already-running `workspacer serve` on the same machine instead of spawning daemons; "Connect to Server…" (palette / Remote Control dialog) points the whole app at a remote server, disconnect relaunches local | 🔵 Working |
| Capability-scoped tokens | `workspacer token create --scope view` / `triage` / `operator` (+ `list` / `revoke`), enforced at the hub's single dispatch path; scoped tokens fail closed on unknown methods; hot-reloaded from `tokens.json`; the legacy remote-token stays implicit operator | 🟢 Solid |
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
| Per-method capability tokens | Enforced: view/triage/operator grant sets at the router's dispatch seam (see §8); the MCP facade itself connects with the host token (operator) | 🟢 Solid |

## 11. Terminal client — `wks-tui` (Rust)

| Feature | What it does | Maturity |
|---|---|---|
| Dashboard + chat | Agent list, transcript & live-terminal views, per-agent tabs | 🔵 Working |
| Spawn / approve / answer / signal | Full agent control from the terminal | 🔵 Working |
| Command palette, vim nav | Ctrl-K fuzzy palette; leader + which-key, Ctrl-w splits, harpoon pins, `/` filter, `:` cmdline, vim counts | 🔵 Working |
| Daemon bootstrap | Auto-spawns claudemon if not running | 🔵 Working |
| Tests | Behavioral unit coverage for navigation, approval/question handling, review pane basics, direct mode, bus mode, and reconnect behavior | 🟢 Solid |

## 12. Substrate — `claudemon` daemon (Rust)

| Feature | What it does | Maturity |
|---|---|---|
| Hook intake | Receives all Claude Code hook events; deferred-hook approval gate | 🟢 Solid |
| Managed provider adapters | `claude_stream` / `codex` / `opencode` / `pi`: pure per-provider `translate()` + shared `apply_updates`, spawned via `POST /sessions/spawn-managed` | 🟢 Solid |
| AskUserQuestion shim | `POST /mcp/ask/:session_id` — a minimal MCP endpoint that parks the session in Question until `/answer`; Codex mounts it as an MCP config override, OpenCode as a remote MCP entry, Pi via a generated extension | 🔵 Working |
| Codex restart durability + resume | Session→thread sidecar under `~/.workspacer/codex-threads`; a restarted daemon lazily replays the thread's rollout into the conversation, and spawn-managed resume rejoins via `thread/resume` | 🔵 Working |
| Session state machine | In-memory per-session mode/state, broadcast over SSE | 🟢 Solid |
| Session/PTY APIs | input/output/stream/message/approve/answer/decide/gate/resize/spawn | 🟢 Solid |
| Transcript + conversation tailer | Parse JSONL, stream deltas with sequence-join | 🟢 Solid |
| Git API | status/diff/numstat/stage/unstage/commit/push | 🔵 Working |
| Signal delivery | SIGINT (Ctrl-C, structural turn-interrupt on managed drivers), **and real SIGTERM/SIGKILL** to sessions | 🟢 Solid |
| Model pricing | Built-in per-model rates (Claude + OpenAI) price every session's cost readout; edit/extend via `~/.workspacer/model-rates.json` (`{"<model-prefix>": {"input": $/M, "output": $/M, "cached_input": $/M}}`, longest prefix wins, hot-reloaded) | 🔵 Working |
| Wrapper protocol | External-process PTY mirroring over WebSocket | 🔵 Working |
| `claudemon watch` TUI | Built-in dashboard + chat TUI | 🔵 Working |
| `claudemon init` | Merge hooks into `~/.claude/settings.json` | 🔵 Working |
| Summarizer | One-shot Haiku summary endpoint | 🔵 Working |

## 13. Platform, packaging & ops

| Feature | What it does | Maturity |
|---|---|---|
| Daemon supervision | Electron spawns claudemon + hub; **auto-restart on crash with backoff** | 🟢 Solid |
| Binary resolution + packaging | Dev/packaged paths; electron-builder bundles daemons | 🔵 Working |
| Bundled `workspacer` CLI | Headless-server launcher ships in the app; "Install workspacer Command" (palette / Settings → Command Line) puts it on PATH | 🔵 Working |
| Standalone server bundle | Releases include `workspacer-server-<os>-<arch>.tar.gz` (zip on Windows): the four binaries + the web build (served at `/app`) + README — extract, `./workspacer serve` | 🔵 Working |
| GPU escape hatch | `WORKSPACER_DISABLE_GPU` for broken Wayland GPU rendering | 🔵 Working |
| Font discovery | Nerd Font scan + custom protocol serving | 🔵 Working |
| Chrome UA spoofing | Strips Electron UA for webviews | 🔵 Working |
| Chrome cookie import | Windows (CDP + direct); macOS/Linux not implemented | 🟡 Partial |
| Toolchains | Pinned via mise (Go 1.25, Node 22) + cargo | 🔵 Working |

---

## Parked / experimental (built, not surfaced)

Decision: the active product is **per-agent workspaces plus the
snapshot-derived Triage Inbox**. The older persisted `/items` inbox is not a
user-facing direction and has no live UI/API consumer. Revive this stack only if
snapshots cannot satisfy a concrete requirement such as durable cross-device
inbox history or supervisor-authored review actions.

| Feature | Notes | Maturity |
|---|---|---|
| Classifier (Stuck/Error/Done + idle) | Internal experimental reference; the shipped Triage Inbox uses renderer snapshot heuristics instead | 🟣 Parked |
| `/items` inbox API + SQLite items store | Internal experimental v2 inbox store; no live consumer | 🟣 Parked |
| SQLite `pending_decisions` / `asks` / `events_fts` | Internal migration/schema leftovers from the v2 inbox direction | 🟣 Parked |
| Terminal emulator (`emulator.rs`) | vt100 emulator with device-query replies; not wired into the output path | 🟣 Parked |

## Known gaps / not built

- Tray icon / taskbar overlay badge.
- macOS/Linux Chrome cookie import.
- Crash-recovery journal (only a `before-quit` save signal today).
- Browser-pane hibernation is renderer-only (no main-process enforcement).

---

## Overall assessment

The **substrate is strong**: the Go hub (bus, RPC, plugins, MCP facade,
supervisor) and the Rust claudemon (hooks, sessions, PTY, transcripts, git) are
complete and well-tested. The **desktop app is feature-rich and cohesive** —
~30 working UI surfaces over a fully-wired main process with solid session-store
test coverage. Recent hardening closed the biggest production risks: daemon
auto-restart, real terminate/kill signals, the cross-client agent-duplication
fix, and — with capability-scoped tokens now enforced at the bus — the last
authorization gap. The app also stands on its own without a desktop: the
`workspacer serve` CLI runs the whole control plane headless, and the Electron
app doubles as a client of it (local adopt or remote connect).

The main *product* ambiguity is resolved: the app is a **per-agent workspace**,
not the abandoned v2 "inbox of decisions" — the leftover classifier/items stack
is now clearly parked as internal experimental substrate with a specific revival
trigger. Remaining work is breadth and platform polish, not core soundness.
