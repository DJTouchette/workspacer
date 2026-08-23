---
title: Mission Control: promoted snapshot store, attention feed, and Inbox/Fleet projections
tags: [renderer-state, mission-control, attention, snapshot-store, viewLevel, resolve-actions]
related_paths:
  - "apps/desktop/src/renderer/src/contexts/AttentionContext.tsx"
  - "apps/desktop/src/renderer/src/hooks/useAttentionFeed.ts"
  - "apps/desktop/src/renderer/src/lib/attentionRouter.ts"
  - "apps/desktop/src/renderer/src/lib/resolveAttention.ts"
  - "apps/desktop/src/renderer/src/types/attention.ts"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Mission Control: promoted snapshot store, attention feed, and Inbox/Fleet projections

## Overview
`App.tsx` promotes every session's full `ClaudeSessionSnapshot` (not just its ambient state) into a single `snapshotBySession` map. `useAttentionFeed` derives a flat, deduped, sorted list of `AttentionItem`s from that map, and `AttentionContext` wraps the feed with selection state, an Inbox tab filter, a `viewLevel` (`fleet` | `piloting`), and cross-agent resolve actions. The Triage Inbox drawer and the Fleet Deck are both pure projections of the same `feed`/`topByAgent`; there is no separate daemon-side attention model (MVP, per the types file's own comment).

## Key modules
- `apps/desktop/src/renderer/src/App.tsx` — owns `snapshotBySession`/`statusBySession` state (~L454-521), `refreshSessionSnapshots` (pull-based hydration on mount + `useHubReconnect`), the `onClaudeSessionUpdate` subscriber, `pruneSession` (~L527-539), and instantiates `useAttentionFeed` (~L1079) which is passed into `AttentionProvider`.
- `apps/desktop/src/renderer/src/hooks/useAttentionFeed.ts` — the feed derivation: builds `approval`/`question`/`done`/`bigdiff`/`stuck`/`error` items per agent, dedupes/sorts them, tracks `dismissed`/`snoozedUntil`, and exposes `topByAgent`.
- `apps/desktop/src/renderer/src/lib/attentionRouter.ts` — pure, side-effect-free `KIND_PRIORITY` table, `sortItems`, and `agentAttentionScore` (Fleet Deck card buoyancy).
- `apps/desktop/src/renderer/src/contexts/AttentionContext.tsx` — `AttentionProvider`/`useAttention()`: inbox filter/selection, `viewLevel`, and the by-`sessionId` resolve actions (`approve`, `answer`, `reply`, `sendMessage`, `openAgent`, `respawn`, `reviewFile`, `spawnAgent`).
- `apps/desktop/src/renderer/src/lib/resolveAttention.ts` — `resolveApproval`/`resolveAnswer`/`resolveReply`: the shared by-`sessionId` resolve path (daemon endpoint first, PTY-keystroke fallback for claude only), with inline documentation of semantics including race guards.
- `apps/desktop/src/renderer/src/types/attention.ts` — `AttentionKind`, `AttentionStatus`, `AttentionPayload`, `AttentionItem` shape (incl. `signature`).
- `apps/desktop/src/renderer/src/components/FleetDeck.tsx` and `apps/desktop/src/renderer/src/components/SideBar.tsx` — the two `topByAgent` consumers (card buoyancy / per-row dot).
- `apps/desktop/src/renderer/src/panes/AgentsPane.tsx` — another `useAttentionFeed`/`topByAgent` consumer (fleet-monitor pane).

## Failure modes
- Snapshot loss: while the hub socket is down, `onClaudeSessionUpdate` ticks are missed; `refreshSessionSnapshots` is re-run via `useHubReconnect` to backfill, otherwise a web/remote tab shows stale sessions until reconnect.
- Missing/absent daemon on first load: `getAllClaudeSessions().catch()` still seeds `preexistingSessionIdsRef.current` to an empty set so agent auto-adoption isn't blocked forever (App.tsx ~L485-490).
- Resolve-action failures degrade to PTY keystroke fallbacks only for `provider === 'claude'`; managed providers (codex/opencode/pi) have no PTY fallback and just log a warning (`resolveAttention.ts` `resolveAnswer`/`resolveApproval`).
- If `snapshot.status === 'ended'` arrives via `onClaudeSessionUpdate`, App drops both `statusBySession[sessionId]` and `snapshotBySession[sessionId]` and calls `stopAgentForSession(sessionId)` — skipping this drop would pin a dead session's full transcript in memory indefinitely.
- `useAgentManager.terminateAgent` does not own the App-level snapshot maps, so `handleTerminateAgent` must explicitly call `pruneSession(sid)` after `terminateAgent(agentId)`, or a manually-terminated agent's snapshot leaks the same way an "ended" one would if unhandled.

## Gotchas
- Item identity is the `signature` string, formatted per-kind from stable fields in `useAttentionFeed.ts`:
  - `approval`: `${sessionId}:approval:${hash(toolName+input)}`
  - `question`: `${sessionId}:question:${hash(questionText)}`
  - `done`: `${sessionId}:done:${doneAt}` (timestamp, not hash)
  - `bigdiff`: `${sessionId}:bigdiff:${fileCount}:${lineCount}` (file/line counts, not hash)
  - `stuck`: `${sessionId}:stuck:${hash(questionText)}`
  - `error`: `${sessionId}:error:${toolCallId}` (tool ID, not hash)
  
  Re-arriving snapshots update items in place; they never duplicate.
- `dismissed`/`snoozedUntil` are keyed by full signature but pruned by session-id *prefix* (`sig.split(':')[0]`) whenever `Object.keys(snapshotBySession)` changes (`liveSessionKey` effect) — a signature format change that stops starting with the raw sessionId silently breaks this pruning and leaks entries forever.
- `done` detection needs cross-render memory: `prevStateRef` (last `ambientState` per session) and `doneAtRef` (timestamp of the working→idle transition) are refs, not state, and are pruned in the same session-liveness effect as the dismissed/snoozed sets — losing that prune means these Maps also grow unbounded.
- The ticker (`now`) only runs (`setInterval`, 5s) when `tickNeeded` (a live snooze in the future OR — if `stuck` is enabled — a pending question that hasn't yet crossed `STUCK_MS` = 5min) AND `usePageVisible()` is true; otherwise the effect is a no-op so the app idles toward ~0% CPU when nothing time-sensitive is pending or the tab is hidden.
- `KIND_PRIORITY` ordering (`approval:100 > question:95 > error:80 > stuck:70 > bigdiff:40 > done:20`) encodes "things that block a human outrank things that are merely happening"; `sortItems` breaks ties oldest-first. `agentAttentionScore` layers `topItemPriority` on top of ambient-state bands (`waiting_approval:900 > waiting_input:880 > thinking:500 > streaming:480 > idle:200 > stopped/unknown:0`) so a card with a live approval/question always outranks a bare working/idle state (`1000 + topItemPriority` dominates every state band).
- `topByAgent` (agentId → single most-urgent open item) is derived by taking the first hit per agent from the already-sorted `items` list — it is the one shared structure consumed by both `FleetDeck.tsx` and `SideBar.tsx`; changing `sortItems`'s tie-break changes which item "wins" a slot in both places simultaneously.
- Piloting-mode auto-dismiss: while `viewLevel === 'piloting'` and an item's `agentId === activeAgentId`, `AttentionContext` dismisses it immediately on surfacing (effect over `feed`) so the currently-piloted agent's own live pane doesn't also duplicate into the Inbox/Fleet — this is gated on `viewLevel`, so Fleet/Inbox triage views still list the active agent's items normally when not piloting.
- `AttentionContext` deliberately splits a `useMemo`'d **stable actions bundle** (`openInbox`, `closeInbox`, `approve`, `answer`, `reply`, `sendMessage`, `dismiss`, `snooze`, `openAgent`, `respawn`, `reviewFile`, `spawnAgent` — all pre-`useCallback`'d) from the volatile data (`feed`, `counts`, `topByAgent`, `selectedItem`, ...); action-only consumers that destructure just the actions object avoid re-rendering on every feed tick. Don't add a non-memoized value directly into `actions`'s dep array without wrapping it, or this optimization silently breaks.
- Every resolve action (`approve`/`answer`/`reply`/`sendMessage`) dispatches by `sessionId` via `window.electronAPI.claudeApprove/claudeAnswer/claudeMessage/claudeWrite` — never by owning a pane's MessagePort — so any agent is resolvable from the Inbox/Fleet without it being the piloted/active pane; `resolveAttention.ts` documents its semantics and warns to keep them in sync with any evolution of approval/answer handling.
- `openAgent()` in `AttentionContext` both clears that agent's items (via a `feedRef` ref, deliberately *not* a `feed` dependency, to avoid rebuilding the stable actions bundle every feed tick) and flips `viewLevel` to `'piloting'` — opening an agent from Inbox/Fleet is itself a triage action, not just navigation.

## Hand-authored notes (2026-07-22)

- **In-app notification center** (bell in NavBar + bottom-right toasts) is a SEPARATE, complementary surface to the attention feed: the feed models *open, resolvable* items (approve/answer live from it); the center is an append-only *history log* (what happened while away). Both key off the same ambient-state transitions but never share state. Spine: main `agentNotifier.postInApp` → IPC `notify:in-app` → `NotificationsProvider` (contexts/NotificationsContext.tsx) → lib/notificationStore.ts (pure, tested) — plus bus `notify.post` events and renderer `lib/notificationBus.ts` posts. `agentNotifier.notifyOnTransition` now also enriches OS bodies (approval tool+input gist from `pendingApproval`, question text from `pendingQuestions`, cost on done) and mirrors every transition in-app with `key: agent:<sessionId>:<needs-you|done>` (same-key replaces), `silent` when the user is watching that session. Config: `notifications.inAppToasts` (default true) gates toasts only; `notifications.enabled` gates ONLY the OS surface — the center always records.
- **Unfocused-window escalation (2026-07-22)**: renderer-ingested notifications (bus `notify.post`, `lib/notificationBus` posts) escalate to OS notifications when `!document.hasFocus()` at ingest — pure rule in `shouldEscalate()` (lib/notificationStore.ts): origin `renderer` only (main-originated ones made their own OS decision — re-raising would double-fire), non-silent only. IPC: `notify:escalate` (renderer→main, `agentNotifier.escalateFromRenderer` re-checks `notifications.enabled`) and `notify:activate` (main→renderer on click → provider's `activate()` marks read + navigates). Web build: `webBackend.notifyEscalate` uses the browser Notification API (lazy permission request, `tag` = key for collapse). Redelivery guard: provider `seenIdsRef` set suppresses re-toast/re-escalate on bus reconnect replays.

## Hand-authored notes (2026-07-19/20) — approvals are transport-shaped

- **Surfacing**: `snapshot.pendingApproval` is fed two ways — hookEventRouter's `PermissionRequest` (PTY Claude) or `claudeSessionStore.applyManagedMode` folding the daemon's `Pending::Approval` slot. The fold gate is `provider !== 'claude' || transport === 'stream'`: stream-Claude approvals arrive via the control protocol (`can_use_tool` in claude_stream.rs), there is NO PermissionRequest hook, so gating the fold on provider alone left stream approvals invisible everywhere (fixed 64cdccb). PTY still defers to the hook path so the two never race. ClaudePane has no inline approval card — approvals surface ONLY via the attention system.
- **Resolving**: `resolveAttention.ts` must track ClaudePane's guards *including transport* — stream-Claude is no-PTY, so `resolveAnswer`/`resolveApproval` thread transport from the snapshot and never take the PTY-keystroke branch for `'stream'` (fixed bdf8ceb; previously answering a stream question from the Inbox silently did nothing). `resolveAttention.ts`'s header names ClaudePane as source of truth — re-check it whenever ClaudePane's approval/answer guards change.
- **Sidebar card activity lines**: `lib/agentActivityLog.ts` `collectRecentActivity()` unions `turns[].toolCalls` (durable, transcript-derived) with the two hook lists (`activeToolCalls`/`completedToolCalls` — fresher mid-turn but CLEARED by applyStopEvent at every turn end), deduped by tool_use id with hook entries winning. Leaning on the hook lists alone renders "Working…" placeholders between turns — the durable copy lives on the conversation turns.

## Hand-authored notes (2026-08-16) — federation

- Remote (hub-stamped) sessions flow through the same snapshot store and attention feed; resolve actions route to the owning hub (main-process action routing / qualified calls), `AttentionCard`/`AgentCard` show the hub chip, and an offline peer leaves tombstones rather than silently dropping its items. See `modules/hub-federation.md`.
