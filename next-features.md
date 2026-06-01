# Workspacer — Next Features

A backlog of missing and good-to-have features, grounded in the current codebase
(Electron + React renderer over the `claudemon` Rust daemon). Ordered roughly by
value-for-effort.

## Context: what already exists

Workspacer runs **multiple long-lived Claude Code agents** side by side. Each agent
has its own cwd, a status dot (idle / thinking / working / needs-approval / stopped),
and a horizontal-scrolling set of tabs/panes. Pane types: terminal, browser, Claude
(terminal *and* GUI mode with approve/answer), tracker (Jira/ADO), devops, settings,
daemon dashboard. There's a command palette, vim/default keybindings, Claude profiles,
session save/restore, and browser-pane hibernation.

Crucially, the daemon already runs a **classifier** (`claudemon/src/classifier`) that
detects per-session states — `NeedsInput`, `Error`, `Stuck` (loop + idle detection),
`Done` — and emits priority-ranked "items" over the `/items` API.

---

## 1. Finish what's half-built (backend exists, UI doesn't)

Highest leverage — most of this code is already written.

- **Build out `NotesPane`** — currently a "Phase 2" placeholder. Lowest-hanging fruit:
  a per-agent markdown scratchpad that saves with the session.
- **Build out `AgentPane`** — also a "Phase 2" placeholder.

> **Triage inbox — dropped (2026-06-01).** The per-agent workspace model plus
> sidebar status/counts + OS notifications + jump-to-next-attention (#2) cover the
> routing a unified inbox would have. Decided not to rebuild it. The classifier's
> `Stuck`/`Error` detections are *not* surfaced anywhere now (the only consumer,
> `InboxPane`, was deleted; `claudemonItems.ts` + `ItemDetailOverlay.tsx` are dead
> code) — revisit only if silent loop/error detection becomes a felt need.

## 2. Notifications & ambient awareness ✅ (shipped 2026-06-01)

The core use case is babysitting agents you're *not* actively watching.

- ✅ **OS notifications + taskbar flash** when an agent needs approval/input or
  finishes (working→idle). Fires from `claudeSessionStore` ambient-state
  transitions via `agentNotifier` (main). Suppressed for the agent you're
  watching (window focused + that agent on screen). Config block `notifications`
  (`enabled`/`notifyDone`/`onlyWhenUnwatched`/`sound`), silent by default. Tray
  badge / overlay icon not done (Windows needs a generated overlay image).
- ✅ **"Jump to next agent that needs me"** — `ctrl+alt+→` (vim chord `<leader> m`),
  also the clickable "N need you" sidebar header.
- ✅ **Aggregate counts** in the sidebar header ("N need you" / "N working").

## 3. Git / diff review

- **Built-in diff viewer.** Agents write code; reviewing it currently means dropping
  to a terminal. The classifier even emits `review_diff` and `merge` as next-actions
  with no UI behind them. A per-agent "what changed" pane (git status / diff / stage /
  commit) would close the loop from "agent done" → "I review & merge."
- **Git worktree integration.** Spawning each agent in its own worktree would let
  multiple agents work the same repo without colliding. The untracked
  `claudemon/src/session/emulator.rs` suggests movement here.

## 4. Agent orchestration

- **Completion triggers / task queue** — let one agent's completion kick off another;
  "run this prompt across N agents." The command palette already labels the
  agent-manager pane "Prompts, runs, workflows" — that's the unbuilt ambition.
- **Saved / templated prompts** or snippets to fire into a session.
- **Cross-agent broadcast** — send one message to all agents.

## 5. Session / workspace management

- **Session rename / duplicate / export**, autosave indicator, crash-recovery prompt
  beyond the picker.
- **Search across agent transcripts.** The daemon stores transcripts
  (`/sessions/:id/transcript`) but there's no UI to search across them.
- **Per-agent cost / token usage** surfaced somewhere.

## 6. Quality of life

- **Custom theme editor.** Only `dark`/`light` presets today, though the `--wks-*`
  token system clearly supports more.
- **Browser pane** polish — address bar / history UI; surface the existing
  Chrome-cookie import.
- **Per-agent default profile / per-project config memory.**

---

## Suggested top 3 (value-for-effort)

1. ~~Bring back a triage inbox~~ — dropped; the workspace + notifications cover it.
2. ✅ **OS notifications** when an agent needs you — shipped.
3. ✅ **A diff / review pane** so "agent finished" leads somewhere — shipped
   (read-only + stage/commit/push; `review_diff`/`merge` wiring still open).
