---
title: Session Lifecycle
tags: [sessions, state, snapshot, lifecycle]
related_paths:
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/renderer/src/types/claudeSession.ts"
  - "apps/desktop/src/main/services/sessionStore/hookEventRouter.ts"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/store.rs"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Session Lifecycle

## Overview
Sessions are created when an agent spawns (via pre-registered spawn UUID in claudemon, then hook-driven state machine) and mutate through hook events. The `SessionState`/`ClaudeSessionSnapshot` dual model represents agents across desktop (Electron main process) and daemon (Rust claudemon), with aliveness tracked separately per transport (PTY hooks vs stream managed mode). Dead sessions linger as resumable `Stopped` rows indefinitely; archiving hides them after 7 idle days but keeps them on disk. Both desktop and daemon apply hook events to their own state machines independently—the stream transport routes state through the daemon's managed driver instead of hooks.

## Key modules
- `apps/desktop/src/main/services/claudeSessionStore.ts` — desktop-side session store; owns IPC pushes to renderer, merges hook/delta/statusline into ClaudeSessionState, evicts ended sessions after 30s grace period.
- `apps/desktop/src/renderer/src/types/claudeSession.ts` — TypeScript snapshot type (ClaudeSessionSnapshot); serializable over IPC, defines status/ambientState/usage/subagent shape.
- `apps/desktop/src/main/services/sessionStore/hookEventRouter.ts` — pure switch logic for hook event application; handles PreToolUse/PostToolUse/Stop/SubagentStart/SubagentStop/PermissionRequest/Compaction; stream sessions skip ambientState mutation (managed driver owns it).
- `services/claudemon/src/session/state.rs` — SessionMode enum and SessionState.apply() method; authoritative state machine for PTY sessions, increments tool_calls, tracks live_subagents for background task draining.
- `services/claudemon/src/session/store.rs` — daemon-side session registry; holds SessionState for all sessions (live and Stopped), broadcasts updates/hooks/statusline on separate channels, manages aliases (hook session_id → spawn UUID), hydrates from SQLite at boot.

## Failure modes
- **Interrupted turns (Esc) send no Stop hook**: conversationApplier detects `[Request interrupted by user]` markers in the transcript batch and synthesizes a stop/idle transition (without this, session sticks on 'streaming'). Subagents spawned before interrupt are left 'running' indefinitely.
- **Stream vs PTY ambientState hazard**: hooks always fire for stream sessions, but `hooksOwnAmbient` guards prevent them from clobbering the daemon's managed mode. Forgetting this guard causes the two state machines to fight—PTY can idle while daemon says streaming.
- **Pending message injection during composer redraw**: messages queued while session isn't `Input` (Unknown/Responding/Approval/Question) are flushed on transition to Input, but only after a 300ms delay (FLUSH_DELAY_MS) to avoid the text landing in the box mid-redraw and the Enter being swallowed. Verify > submit timing hazard mitigated by tracking input_since and client_input_at.
- **Session eviction race**: desktop's 30s grace period post-SessionEnd can race with daemon's session entry lingering (no timeout, kept as Stopped). Desktop aliveness filtering (`status !== 'ended'`) is the authoritative view; daemon is consulted only for resumption.
- **Managed (Codex/OpenCode) approval/question cards**: these sessions fire no Claude hooks, so the daemon's pending slot (ApprovalMode/Question frames) is the only source for the UI. Must not be race-driven by hooks (which don't touch them for managed sessions).

## Gotchas
- **ClaudeSessionSnapshot == ClaudeSessionState**: the type is a direct alias; desktop's state extends the snapshot's shape with backend fields (transcriptPath, startedAt, peakContext, isSupervisor). Snapshots pushed to renderer must omit these—currently done via spread operator, not a Omit<> type.
- **Tool call idempotency by tool_use_id**: re-delivered PreToolUse (e.g. after SSE reconnect) must not spawn a second card. Guard checks `activeToolCalls + completedToolCalls` by id before pushing; forgot this and active cards pile up.
- **Subagent file changes always recorded**: even though subagent tool cards don't appear in the main chat's work log (filtered by agent_id), their Edit/MultiEdit/Write file changes ARE added to session.fileChanges—verify this is the intended behavior before refactoring.
- **Subagent background drain on Stop**: when parent's own turn ends (Stop fires) while background (async Agent/Task) subagents are running, session holds 'streaming' until the last SubagentStop, then idles. Track with `live_subagents` counter + `parent_turn_ended` flag. A new UserPromptSubmit resets both.
- **Status vs mode naming split**: desktop uses ambientState; daemon uses mode (SessionMode enum). They must stay in sync; desktop's applyStopEvent/applySessionEndEvent are the only side-effect-free equivalents to daemon's apply().
- **ARCHIVE_AFTER_SECONDS = 604800 (7 days)**: stale stopped sessions hide from the list but stay resumable on disk. The is_archived() check in the daemon is purely a display filter—no automatic deletion.
- **Alias map resilience**: daemon maps Claude's hook session_id to spawn UUID via pending_spawns_by_cwd (first SessionStart) or retains the alias map entry forever. Old aliases from crashed spawns aren't cleaned; this is acceptable because they only bind if Claude re-uses that id with the same cwd, which is rare. Watch for unbounded alias map growth if spawn-id reuse becomes frequent.

## Hand-authored notes (2026-07-19/31) — resume + restart hardening

- **Resume race (fixed 2026-07-19)**: claudemon's `register_spawn` reuses an existing row on resume but used to leave `mode=stopped` until the SessionStart hook landed seconds later — and the desktop's `verifyAttachTarget` (2026-07-18 hardening) treats mode=stopped as dead, so every RECENT resume/respawn lost the race (viewer torn down, "[Claude session exited]" banner, deaf pane). Now: `register_spawn` flips Stopped→Unknown (clears pending, bumps updated_at) like `register_managed` already did; `verifyAttachTarget` only tears the viewer down on 404 — for mode=stopped it fires terminal:exit but KEEPS the SSE viewer attached, because a respawn revives the SAME session id and the still-open stream is what brings the pane back without a remount. `exitNotified` on SessionStream dedupes the double banner.
- **Session ids are REUSED on restart; teardown must prove generation ownership (fixed 2026-07-31).** Every restart path pins the id it replaces, and closes are fire-and-forget, so the dying life's teardown routinely runs AFTER its successor registered under the same id. Three paths keyed on id alone used to clobber the successor: claudemon `reap_pty` (SIGKILLed the successor's PTY), `deregister_managed` (wiped channels/conversation, broadcast SessionEnd), and the desktop's 30s SessionEnd eviction (took label/parent/isSupervisor/usage) — trigger was the routine composer model/permission-mode switch. Fix: `SessionStore` holds a monotonic `generations` DashMap — `claim_generation()` at every spawn path; `owns_generation()` checked by release_spawn/drop_pending_spawn/deregister_managed (an UNCLAIMED id answers true on purpose, else teardown would leak); `reap_pty_owned` uses `remove_if` + `Arc::ptr_eq` (the reader must reap its own child even when superseded). Desktop holds eviction timers in `evictionTimers` so any restart signal (setSpawnMeta is the earliest) cancels them, plus a `status !== 'ended'` re-check in the callback. Any NEW teardown path keyed on session id must take the same generation guard.
