# TUI → Electron Parity Roadmap

Tracking doc for closing the gap between `wks-tui` (`apps/tui`, Rust/ratatui) and
the Electron desktop app. The TUI talks **only to claudemon's REST + SSE** on
loopback (the hub-bus `agents.*`/`claude.*` capabilities are registered by the
Electron main process and absent when it isn't running), so every feature here is
gated on whether claudemon exposes the data. That gating is called out per item.

Status legend: ☐ todo · ◐ in progress · ☑ done · ✗ won't do (out of scope for a terminal)

---

## Current state (what the TUI already does)

Sidebar + Dashboard · per-agent tabs (Claude + shell) · raw PTY terminal ⇄ parsed
transcript toggle (`t`) · attach/detach (`i`/`Ctrl-]`) · spawn modal (`c`) ·
approvals (`y`/`n`/`a`) · questions (`1`–`9` + free text) · messaging · signals
(`x`/`X`) · command palette (`Ctrl-K`) · library (run/insert) · warm background
terminals · reconnect. Core agent **interaction** is at parity.

Architecture recap (so plan items map to code):
- `src/claudemon.rs` — hand-rolled REST+SSE client (`Claudemon`); add one method per new endpoint.
- `src/types.rs` — wire/domain types + transcript parsing.
- `src/app/mod.rs` — `App` state, `View` (`List` | `Agent{id}`), `Workspace`/`Tab`
  (`TabKind::{Claude,Shell}`), `ChatMode::{Terminal,Transcript}`, `AppMsg` pump.
- `src/app/input.rs` — key handling. `src/app/tasks.rs` — async fetchers → `AppMsg`.
- `src/ui.rs` — all rendering.

The extension pattern is consistent: **add an endpoint method → add an `AppMsg`
variant + fetcher in `tasks.rs` → hold the result in `App` → render in `ui.rs` →
bind keys in `input.rs`.** New views fit either as a new `TabKind` or a new
`ChatMode`.

---

## claudemon endpoints not yet used by the TUI

Confirmed from `services/claudemon/src/daemon/api.rs`:

- `/git/status`, `/git/diff`, `/git/numstat` (GET) · `/git/stage`, `/git/unstage`,
  `/git/commit`, `/git/push` (POST) — **full git surface, unused.**
- `/statusline/stream` (SSE) — Claude's authoritative statusLine: model, context
  used %, cost, 5h/7d rate-limit windows. TUI currently derives these from
  transcript usage instead.
- `/sessions/:id/conversation` + `/conversation/stream` — richer than
  `/transcript` (tool results, work-log structure).
- `/sessions/:id/summarize` (POST) — one-shot Haiku summary of a session.
- `/sessions/:id/decide`, `/sessions/:id/gate` — deferred-hook approval gate
  (finer-grained than `/approve`).
- `/items`, `/items/stream`, `/items/:id/action` — the **parked** classifier
  inbox. The shipped desktop UI uses live snapshot state, not this. Skip unless we
  revive it deliberately.

**Not in claudemon REST** (Electron main process / separate store): analytics
SQLite history, layout templates, settings persistence, plugin management, notes
persistence. These need either a new claudemon endpoint or a TUI-local store.

---

## Phase 1 — Git review/diff (flagship gap, fully backed) ☑

The single biggest desktop feature the TUI lacked, and claudemon exposes the whole
API. Self-contained. Done.

- ☑ `Claudemon` methods: `git_status`, `git_diff`, `git_stage`, `git_unstage`,
  `git_commit`, `git_push` (cwd/path as percent-encoded query params; a small
  `encode()` helper). `git_numstat` left for the Phase 5 inspector strip.
- ☑ `types::FileStatus` (path, orig_path, staged/unstaged codes, untracked).
- ☑ Surfaced as a **modal over the agent view** (`App.review: Option<ReviewState>`,
  opened with `R` from the sidebar or an agent — `Action::OpenReview`) rather than
  a tab, to avoid entangling the terminal/tab lifecycle: left = file list (j/k),
  right = colourised unified diff (scroll J/K · Ctrl-d/u), `t` toggles
  staged⇄unstaged.
- ☑ Mutations: stage/unstage (`s`/`u`), stage-all (`a`), commit (`c` → composer),
  push (`P`); failures toast the git error.
- ☑ Tests: `FileStatus` parsing (via serde), `encode()` query-escaping. (Live
  diff round-trip is covered by claudemon's own git tests.)
- ◐ Remaining nicety: a command-palette "Review changes" entry (currently `R` only).

## Phase 2 — Authoritative statusline

Consistency win mirroring the desktop `deriveSessionStats` fix (statusLine first,
transcript usage fallback). Done.

- ☑ Subscribe to `/statusline/stream` (`claudemon::spawn_status_lines`, own
  reconnect loop); hold latest `StatusLine` per session on `App.status_lines`.
- ☑ `types::derive_stats(agent, statusline?)` — statusLine context%/cost/model
  first, transcript `Usage` fallback. Used in the sidebar, detail, and dashboard.
- ☑ 5h/7d rate-limit windows on the Dashboard (colour-coded) + the detail pane.

## Phase 3 — Agent management ☑ (reorder deferred)

- ☑ Terminate (SIGTERM via `X`) + **respawn** (`S`) — spawns a fresh Claude in a
  stopped agent's cwd with the default profile (claudemon assigns a new session
  id; the old stopped one lingers until pruned — the TUI can't re-point a card
  the way the desktop does).
- ☑ **Rename** (`e`) — a per-cwd custom name overlay, persisted to
  `~/.config/workspacer/tui-names.json` (`names.rs`), surfaced in the sidebar,
  chat/terminal title, dashboard roster, and palette. Keyed by cwd so it
  survives a respawn / daemon restart.
- ✗ Sidebar reorder — **deferred**: it would fight the attention-first sort
  (waiting agents float to the top), which is a core part of the dashboard UX.

## Phase 4 — Notes & overview ☑ (MRU deferred)

- ☑ **Notes pane** — per-agent markdown scratchpad opened with `N` (a modal,
  not a tab — consistent with review/rename), edited in-TUI (append-style:
  type / newline / backspace; `esc` saves), persisted per cwd to
  `~/.config/workspacer/tui-notes.json` (`notes.rs`). No mid-text cursor yet
  (it's a scratchpad); a full editor is a later nicety.
- ☑ **Dashboard rate-limit card** — done in Phase 2.
- ◐ Recent/favourite spawn dirs (MRU) — deferred; the spawn modal's path
  completion already covers most of this.

## Phase 5 — Inspector & richer conversation ☑

- ☑ Switched the parsed view from `/transcript` to `/conversation`
  (`turns_from_conversation`): a `tool_result` attaches to its `tool_use` by id,
  so tool **output** renders inline as a dimmed `↳` snippet (red on error).
  Consecutive same-role items coalesce into turns; injected meta is filtered.
- ☑ Inspector strip on an open agent's tab bar: `⎇ branch ±changed` (cheap
  `git_status` fetched on open, cached per cwd via `AppMsg::GitSummary`).
- ✗ `summarize` action — **dropped**: the Haiku summarizer was removed entirely
  (it spawned phantom sessions); there's nothing to call.

## Phase 6 — Test depth & polish

Today only terminal key-encoding + type-parsing units exist (`features.md` flags
the TUI suite as 🟡 Partial).

- ☐ App-state tests for each new view (review, notes, statusline precedence).
- ☐ Error/empty states (no git repo, detached session, daemon down) rendered
  cleanly rather than blank — the terminal analogue of the desktop ErrorBoundary
  /EmptyState pass.

## Phase 7 — Settings (config: themes + keybindings) ☑

Done ahead of the rest at the user's request — the desktop app has themes +
remappable keybindings + a shortcut overlay, and the TUI now matches.

- ☑ `~/.config/workspacer/tui.json` loader (`config.rs`) — optional,
  all-defaulted, warns-and-degrades on a bad file.
- ☑ **Theme** (`theme.rs`) — semantic color roles, built-in presets
  (default/nord/gruvbox/ansi), per-role overrides; threaded through `ui.rs`.
- ☑ **Keybindings** (`keys.rs`) — semantic `Action` enum + per-context `Keymap`
  with config overrides (`none` to unbind); `input.rs` dispatches through it.
- ☑ **Help overlay** (`?`) — lists active bindings per context + themes,
  generated from the live keymap so it can't drift.
- ☐ Remaining: persist runtime changes (live theme/keymap reload), a settings
  *editor* in-app (vs editing JSON), and notification config.

---

## Explicitly out of scope (desktop-only / no terminal analogue)

✗ Spatial / stacked view modes · ✗ Fleet Deck radar · ✗ browser/plugin webview
panes · ✗ OS notification config UI (a terminal bell on needs-input is the most
we'd add) · ✗ analytics history (no claudemon endpoint; would need new backend)
· ✗ layout templates.

> Themes moved **out of "out of scope" into Phase 7** — the user asked for them.

---

## Log

- 2026-06-13 — Created. Gap analysis from `docs/features.md` + a read of the TUI
  source and claudemon's route table. Baseline `cargo build` clean; 76 desktop
  tests + renderer typecheck green (UI/UX hardening sweep pushed in the same
  session).
- 2026-06-13 — Phase 7 (Settings) done at the user's request: config-loaded
  theme system (`theme.rs`/`config.rs`), configurable keybindings (`keys.rs`,
  `input.rs` refactored to keymap dispatch), and a `?` help overlay. 60 TUI
  tests pass; clean build. Config documented in `apps/tui/README.md`.
- 2026-06-14 — Phase 1 (git review pane) done: claudemon git client methods +
  `FileStatus`, `ReviewState` modal opened with `R`, file list + colourised diff,
  stage/unstage/all/commit/push. Also coalesced consecutive tool-only turns in
  the transcript into one compact line (matches the desktop WorkCard). 64 TUI
  tests pass; clean build.
- 2026-06-14 — Phase 2 (authoritative statusline) done: `/statusline/stream`
  subscription, `StatusLine` per session, `derive_stats` precedence wired into
  sidebar/detail/dashboard, 5h/7d rate-limit windows. 66 TUI tests pass.
- 2026-06-14 — Phase 3 (agent management) done: rename (`e`, persisted per-cwd
  via `names.rs`) + respawn (`S`) of stopped agents (extracted `spawn_agent_in`).
  Sidebar reorder deferred (conflicts with attention sort). 66 TUI tests pass.
- 2026-06-14 — Phase 4 (notes) done: `N` per-cwd markdown scratchpad modal
  (`notes.rs`), append-style edit, persisted. Verified live via tmux (open/edit/
  save/persist round-trip). MRU spawn dirs deferred. 66 TUI tests pass.
- 2026-06-14 — Phase 5 done: parsed view switched to `/conversation`
  (`turns_from_conversation`) so tool results render inline (`↳`); tab-bar git
  inspector (`⎇ branch ±changed`). summarize action dropped (summarizer removed).
  Verified live via tmux (tool output + inspector ±N). 68 TUI tests pass.
