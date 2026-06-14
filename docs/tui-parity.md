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

## Phase 1 — Git review/diff (flagship gap, fully backed)

The single biggest desktop feature the TUI lacks, and claudemon exposes the whole
API. Self-contained.

- ☐ `Claudemon` methods: `git_status`, `git_diff`, `git_numstat`, `git_stage`,
  `git_unstage`, `git_commit`, `git_push` (cwd as query param).
- ☐ Types for status entries (path, staged/unstaged/untracked) + parsed unified diff.
- ☐ Surface as a **`TabKind::Review`** (or a `ChatMode` on the Claude tab — decide):
  left = file list (j/k, staged/unstaged sections), right = unified diff with
  scroll; syntax-free first, colourised +/- lines.
- ☐ Mutations: stage/unstage (`s`/`u`), commit (`C` → message composer), push (`P`).
- ☐ Command-palette entry "Review changes" + a key from the agent view.
- ☐ Tests: status/diff parsing, stage/unstage state transitions.

## Phase 2 — Authoritative statusline

Consistency win mirroring the desktop `deriveSessionStats` fix (statusLine first,
transcript usage fallback).

- ☐ Subscribe to `/statusline/stream`; hold latest `StatusLine` per session.
- ☐ Prefer statusLine context%/cost/model in the sidebar + dashboard; keep
  transcript `Usage` as fallback when statusLine absent.
- ☐ Surface 5h/7d rate-limit windows (Dashboard footer / agent header).

## Phase 3 — Agent management

- ☐ Terminate (already have SIGTERM via `X`) + explicit **respawn** of a stopped
  agent (re-spawn with the same argv/cwd).
- ☐ **Rename** — local display-name overlay (TUI-side map; no daemon support
  needed). Persist to the TUI config dir.
- ☐ Sidebar reorder (optional; local ordering).

## Phase 4 — Notes & overview

- ☐ **Notes pane** — per-agent markdown scratchpad as a `TabKind::Notes`, edited
  in-TUI, persisted to the TUI config dir keyed by cwd (desktop persists per
  session; we approximate by cwd since that's the stable agent identity).
- ☐ **Overview/Dashboard upgrade** — rate-limit card (from Phase 2 statusline),
  recent/favourite dirs to spawn into (from a TUI-local MRU of spawn cwds).

## Phase 5 — Inspector & richer conversation

- ☐ Switch the transcript path to `/conversation` for tool-result / work-log
  fidelity (inline diffs, tool output) closer to the desktop GUI pane.
- ☐ Inspector strip: files-changed count (git numstat) + usage, shown alongside
  an open agent. (Workflows/subagents telemetry is **not** in claudemon REST —
  out of scope until it is.)
- ☐ `summarize` action — palette/key to drop a Haiku summary into the view.

## Phase 6 — Test depth & polish

Today only terminal key-encoding + type-parsing units exist (`features.md` flags
the TUI suite as 🟡 Partial).

- ☐ App-state tests for each new view (review, notes, statusline precedence).
- ☐ Error/empty states (no git repo, detached session, daemon down) rendered
  cleanly rather than blank — the terminal analogue of the desktop ErrorBoundary
  /EmptyState pass.

---

## Explicitly out of scope (desktop-only / no terminal analogue)

✗ Spatial / stacked view modes · ✗ Fleet Deck radar · ✗ themes (terminal owns its
palette) · ✗ browser/plugin webview panes · ✗ OS notification config UI (a
terminal bell on needs-input is the most we'd add) · ✗ analytics history (no
claudemon endpoint; would need new backend) · ✗ layout templates.

---

## Log

- 2026-06-13 — Created. Gap analysis from `docs/features.md` + a read of the TUI
  source and claudemon's route table. Baseline `cargo build` clean; 76 desktop
  tests + renderer typecheck green (UI/UX hardening sweep pushed in the same
  session).
