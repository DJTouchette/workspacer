---
title: Session Lifecycle
tags: [sessions, state, snapshot, lifecycle, clear, manager]
related_paths:
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/renderer/src/types/claudeSession.ts"
  - "apps/desktop/src/main/services/sessionStore/hookEventRouter.ts"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/store.rs"
owner: Damien Touchette
last_reviewed: 2026-09-01
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

## Hand-authored notes (2026-08-23) — the pending-slot ownership invariant

- **Invariant: exactly one feed owns a session's pending slot** (`pendingApproval`/`pendingQuestions`); others may enrich a row but must never clear or park on a slot they don't own. This was rediscovered independently four times in one day, each census finding MORE writers than the last (2→6, 2→3, 2→6) — that escalation pattern is itself the lesson: grep for direct field writes, don't trust a prior count. The final census on `apps/desktop/src/main` found FOUR: (1) the hook feed (`hookEventRouter`), (2) the daemon feed (`claudeSessionStore.applyManagedPending`), (3) the **federation** feed (`upsertRemoteSession`/`upsertSparseRemoteSession`/`sparsePendingApproval`, which rebuild a row wholesale from a peer's wire snapshot), (4) `clearPendingQuestions` (`ipc.ts` CLAUDE_ANSWER, `hubCapabilities.ts` `claude.answer`) — an optimistic clear that needed its own vocabulary word since the hook feed's Park/Resolve pair doesn't have one for "the user answered and the owner accepted it." (3) and (4) were the ones every earlier count missed. Ownership used to be computed from provider/transport alone (`provider !== 'claude' || transport === 'stream'`), which says "hooks" even for a peer's claude/PTY row — so a local feed could clear or park a card that only MIRRORS a request owned by another machine. Now enforced through one module, `apps/desktop/src/main/services/sessionStore/pendingSlot.ts`: `pendingSlotOwner()` (checks `session.hub` first, returns `'federation'` for any mirrored row), `PendingSlot` (park/resolve gated on the declared feed), `acknowledgeAnswer()` (ungated, questions only — an approval always routes through `claude.approve` since the owner may still reject an unknown decision), and `bornWithPending`/`bornWithEmptyPending` for construction. The fence is enforced at the TYPE level, file-local to the store: `private sessions = new Map<string, PendingFencedSession>()` (a `readonly`-narrowed type) fences the entire class body in one line. Two TypeScript facts made this work and will bite again elsewhere: **`readonly` is invisible to assignability** — a fenced session still passes anywhere a full `ClaudeSessionState` is wanted (notifier, watcher, publishSnapshot), so the fence costs zero call-site churn (TS2540 on a bare assignment attempt) — but **`readonly` does NOT fence object-literal construction**, since naming the field in a literal is allowed; the federation feeds rebuild rows wholesale exactly that way, so construction needs a SEPARATE type (`Omit<ClaudeSessionState, 'pendingApproval'|'pendingQuestions'>`, TS2353 if a literal of that type names either field). Note the fence is file-local: `sessions.values()` still hands live rows out to `supervisorNudge` and the notifier, which only read the slot today — a future write there needs the same fence.

## Hand-authored notes (2026-08-23) — manager succession is memory-only

- **A dead manager's identity (`isSupervisor`/`label`/`parentSessionId`) exists ONLY in the Electron main process.** claudemon's session model has none of these fields (`parent_session_id`/`is_supervisor` do not exist in `services/claudemon/src`); the desktop sets all three at spawn time (`setSpawnMeta` in `claudeSpawn.ts`/`managedSpawn.ts`), and the Go brain's own `metaStore` (`services/hub/cmd/brain/enrich.go`) is likewise process memory. So the whole fleet parent/manager graph dies with the Electron process — an app restart leaves NO live row carrying a `parentSessionId` at all. This settles "should a dead-manager tombstone persist to disk": it shouldn't, not yet — a persisted tombstone would name a dead manager nothing points at (no orphans survive a restart to ask about it either). Persistence only becomes worth building in the same change that makes parentage itself durable (a claudemon-side field, or a desktop sidecar rehydrated at boot). Until then, tombstones live in `claudeSessionStore` memory, written at the single teardown path (`evictNow`, so SessionEnd-eviction and `close_session` both feed it), retained while something they parented is still live, capped as a backstop.
- **What the live `parentSessionId` field CAN and can't do for a crash (no handoff file).** Since 2026-08-23 the desktop's `agents.list` row carries `parentSessionId`, so a successor can group live rows by parent and treat any parent id with no row of its own as a dead parent with live children — exactly the `fromSessionId` `adopt_workers` needs. It CANNOT decide which dangling parent was *its own* predecessor: `claudeSessionStore` evicts an ended session ~30s after SessionEnd, so the dead manager has no row at all (no label, no isSupervisor, no cwd) — nothing distinguishes "a dead manager" from "a dead worker that happened to have subagents," or one dead manager from another. With exactly one dangling parent this is usually right; with more than one it's a judgement call (compare live children's cwd/label against what the successor was told to take over) — narrowed, not closed. `adopt_workers` is deliberately NOT federated (`services/hub/cmd/mcp/main.go`): a worker and its manager always share a hub, so a remote row's `parentSessionId` names a session on the PEER and is only adoptable there.
- **`agents.list` is not one shape.** The desktop's `agents.list` (`hubCapabilities.ts`) hand-builds a reduced row; the headless brain's `agents.list`/`sessions.snapshots` (`services/hub/cmd/brain/handlers.go`) answer with the SAME full enriched claudemon snapshot (`visibleSnapshots` run through `enrichAndCompat`) — brain rows have carried `label`/`parentSessionId`/`isSupervisor` all along (pinned by `parity_test.go` `TestEnrichSnapshotCoversMobileNestingFields`, since `/m` nests the fleet on them). So "add a field to `agents.list`" is desktop-only work in practice, and the drift direction is the desktop lagging the brain, not leading it. Before widening the desktop's `agents.list`, check whether the brain already emits the field and whether the desktop's own `sessions.snapshots` (which spreads the whole snapshot) already ships it — both are the same view-authorization tier (`authtoken.go` `viewMethods` admits both equally), so putting the field on the lighter row widens no security boundary.
- **New-capability checklist, three languages must agree**: `hubCapabilities.ts` registration + `capspec.go` (`inertMethods`/`unscopedByDecision`) + renderer `pluginPermissions.ts` `CAP_LABELS`. The renderer suite (`pluginPermissions.test.ts`) parses `hubCapabilities.ts` and fails if a registered capability has no label, independently of whether the main-process suite is green — three suites in three languages must all pass, not just the one you touched.

## Hand-authored notes (2026-08-17) — History pane is transcript-backed; daemon rows remain the resumability truth

- **The Sessions ("History") pane is not driven by the analytics store.** `session_history` (SQLite) is an ANALYTICS table — names/cost — never the resumable list; daemon `GET /sessions` rows are the sole source of truth for what can actually be resumed. Since the 2026-08-17 rewrite, pane content is per-project `claude` transcript listings (`claudeListSessionsForDir`, one call per registered project from `lib/projectRegistry`) merged with daemon rows in `renderer/src/lib/sessionHistoryGroups.ts` (tested); daemon rows remain the only source for managed providers and unregistered directories (trailing "Other directories" group). A transcript-only row resumes via a synthetic `RecentAgentSession` with `transport: 'pty'` ("no recorded choice" — the daemon decides transport at spawn). `App.tsx` threads `historyExcludeIds` (open layout ids + daemon non-stopped rows) so a transcript row can never double-drive an already-live session.
- **`TITLE_ENRICH_LIMIT = 40`** (`apps/desktop/src/main/services/recentSessions.ts`) title-enriches only the first 40 rows of a query, even though the pane itself is uncapped (`App.tsx` passes `Infinity` to `filterResumableSessions`) — rows past 40 fall back to name/dirname labels. The 2026-08-17 rewrite superseded this concern for Claude rows specifically (transcripts carry their own summaries), but the cap still governs daemon-only row titles (managed providers, unregistered dirs).

## Hand-authored notes (2026-08-23/28) — the pending slot, part two: claudemon's own writers and the readers

The 2026-08-23 census above covered `apps/desktop/src/main`. Two more halves
followed, and the escalation continued exactly as predicted.

- **claudemon's pending slot has TWO writers on codex/opencode/pi, not one — and
  fencing it surfaced a THIRD.** The "exactly one feed owns the slot" invariant
  was false for the three managed non-claude providers: `daemon/mcp_ask.rs` (the
  AskUserQuestion MCP shim, registered for codex via `--config mcp_servers`,
  opencode via `ask_mcp_entry`, pi via `ask_extension_source`) parks and resolves
  the same slot the driver task uses for approval cards. Both of its writes were
  unattributed because `PendingWrite` fenced only `Keep` — so a question raised
  beside a parked approval overwrote the card, and `QuestionGuard::finish/drop`
  then cleared the slot, while the driver's FIFO still held the approval and only
  re-surfaces a queued card from its `/approve` decision arm. **Nothing could
  answer it.** Verified by test (both assertions read "pending is None" before the
  fix). Fixed by giving `PendingWrite::Park`/`::Resolve` a `PendingOwner`
  (`Primary` = hook feed or driver task, `Ask` = the shim): a foreign park
  DISPLACES the card and it is restored when the displacing request is released;
  a resolve clears only what its own feed raised. Making the slot private
  (`SessionState::pending_card`, read via `pending()`, written only via
  `write_pending`) is what surfaced the third writer nobody had counted —
  `SessionStore::park_decision`, the PTY hook gateway.
  Practical rules: **never count slot writers by reading doc comments**; grep for
  writes and ask who else can raise a user-blocking request for that provider.
  New writes go through `write_pending` with an explicit `PendingOwner` (the field
  is private, so the compiler makes you). And "one feed" is not "one request" — a
  codex session can legitimately hold an approval AND a question at once, so a
  driver must take `cur_mode` from what `set_managed_mode` RETURNS, since
  releasing its own card can leave the session parked on the other feed's.
- **Fencing past the store's FILE boundary needs `readonly T[]`, not `readonly` —
  and losing assignability back to `ClaudeSessionState` is the feature.** The
  reader-side census (everything OUTSIDE `claudeSessionStore.ts` handed a LIVE row
  via `this.sessions.values()`/`.get()`) found SIX, where the dispatch named two:
  `agentNotifier.notifyOnTransition`, `supervisorNudge.onBlock`/`broadcastBlock`/
  `onFinished`, `SessionUsageAccumulator.applyUsage` + static
  `refreshContextLimit`, `conversationApplier` (4 entry points),
  `budgetWatcher.checkBudget`, `analyticsWriter.writeHistory`. Everything else is
  either internal to the class body or already a structural `Pick` that excludes
  the slot.
  Two facts the earlier `PendingFencedSession` work did not have to confront:
  (a) **`readonly pendingQuestions: PendingQuestion[]` stops
  `session.pendingQuestions = null` (TS2540) and NOTHING else** — `.push()`,
  `.splice()`, `.length = 0` and `[0] = …` all reach the store's own array.
  Stopping those needs `readonly ReadOnlyPendingQuestion[]`, and the payloads need
  freezing too or `card.toolInput.command = …` still rewrites the live card. The
  new `PendingReadOnlySession` (`sessionStore/pendingSlot.ts`) does both.
  (b) **The cost is that `PendingReadOnlySession` is NOT assignable back to
  `ClaudeSessionState`** (a `readonly T[]` is not a `T[]`) — the exact property
  the previous worker relied on for zero call-site churn. That turns out to be
  desirable (a fenced collaborator cannot launder the row by passing it to
  something mutable), but it means the fence widens in exactly ONE place:
  `PendingSlot`'s constructor takes `PendingReadOnlySession` and casts internally.
  Forced by a real case, not a hypothetical: `conversationApplier` is NOT a pure
  reader — its interrupt path calls `hookEventRouter`'s `applyStopEvent`, which
  legitimately clears the slot through a gated `PendingSlot('hooks')`.
  Separately, **`getSnapshot`/`getAllSnapshots` shallow-cloned and the alias was
  live**: `snap.pendingApproval.toolName = 'Read'` changed the store's card, and
  `snap.pendingQuestions.length = 0` unblocked a still-blocked session. Fixed by
  COPYING (`detachPendingSlot`) rather than typing — `ClaudeSessionSnapshot` is
  declared THREE times (`claudeSessionStore.ts`, `main/shared/ipcTypes.ts`,
  `renderer/src/types/claudeSession.ts`), so a readonly type would have had to
  land in all three and would still only bind code that opts into it, whereas a
  caller mutating a detached copy simply cannot reach the store.
  Rules: a collaborator outside the store takes `PendingReadOnlySession`, never
  `ClaudeSessionState`; a legitimate write goes through `PendingSlot`; anything
  leaving as a snapshot goes through `detachPendingSlot` (`webContents.send` does
  not need it — Electron structured-clones). Guards live in
  `apps/desktop/src/main/services/pendingSlotBoundary.test.ts`.

## Hand-authored notes (2026-08-23/27) — liveness and busy/idle are NOT what they look like

- **Only Claude-on-PTY has a heartbeat status line.** There are two entirely
  different sources for a session's `status_line` and only one ticks on a clock.
  (1) HEARTBEAT — Claude Code's own `statusLine` command, re-run on every render
  by the interactive CLI; claudemon installs a forwarder into settings.json
  (`daemon/init.rs` `status_line_command`) that POSTs to `/statusline`, where
  `ingest_status_line` stamps a fresh `received_at`. **Requires provider `claude`
  + transport `pty`.** (2) ACTIVITY-DRIVEN — everything else: codex, opencode, pi,
  **AND claude on the `stream` transport, which is the shipped default.** The line
  is synthesized by `UsageAcc::status_line()` (always stamping
  `received_at: Some(now)`) and published only from `apply_updates`'s
  `if usage_changed`, set by Usage / RateLimits / RateLimitStatus / Capabilities /
  Effort frames. **No timer calls `apply_status_line` anywhere.**
  Two corollaries that are easy to get wrong: `set_account_usage` (gated on
  `provider == "claude"`) is NOT the claude heartbeat — it pushes a patched line
  to live claude sessions but `patch_rate_limits` only stamps `received_at` when
  it is already `None`; and `AgentUpdate::Busy`, described as "a liveness ping",
  is emitted at turn/tool boundaries (`turn/started`, `task_started`,
  `agent_start`), not periodically, and `apply_updates` debounces it away when the
  mode is unchanged.
  **Consequence:** for an activity-driven session `statusLine.receivedAt` cannot
  separate "alive but quiet" from "process gone" — it freezes exactly when
  observable work does. And since `progressFingerprint` (renderer
  `lib/stallDetector.ts`) counts the same token totals the status line carries, a
  fingerprint frozen for STALL_MS *implies* a `receivedAt` frozen for STALL_MS,
  well past the 90s SILENT_MS — so **any aliveness check keyed on it returns
  "dead" 100% of the time it is consulted.** This shipped as a live bug: the
  attention feed's stall card said "No signal — the agent has stopped reporting at
  all" for EVERY codex/opencode/pi stall and every claude/stream stall, making the
  card's "Not moving" half unreachable for the default transport. Fixed 2026-08-23
  (da5e8710) by replacing `StallVerdict.alive: boolean` with
  `signal: 'alive' | 'silent' | 'unknown'`, where anything not heartbeat-backed
  reports `unknown` and the card says so instead of guessing. Two comments
  (`stallDetector.ts`, `mobile.html`) asserted managed providers publish NO status
  line at all — also false, and the reason the wrong fallback looked safe.
  **Before treating any single field as liveness, check which source feeds it:
  `(provider ?? 'claude') === 'claude' && transport !== 'stream'` is the only
  heartbeat-backed case.** A genuine per-session liveness signal for managed
  providers has to be BUILT (a periodic tick in claudemon calling
  `apply_status_line` for sessions holding `managed_inputs`); nothing client-side
  can synthesize it. The desktop's `stallDetector.ts` and the PWA's copy in
  `services/hub/cmd/hub/mobile.html` are identical implementations and must change
  together. Also note real managed-process death never reaches a stall check at
  all: the driver task exits, `deregister_managed` marks the row Stopped, and
  `sweep_ghost_sessions` explicitly skips any session still holding
  `managed_inputs` plumbing.
- **Subagent activity flaps have TWO roots, one per direction — and neither is
  recency.** Diagnosed 2026-08-27 against a live daemon: busy/idle is not derived
  from output-stream recency anywhere (hook path, stream driver and
  `hookEventRouter` are all explicit lifecycle-event machines).
  **FALSE-ACTIVE (parent/child aggregation).** Reproduced live: a codex session sat
  at `mode:input` with `background_tasks:1` and one `subagents[]` row at
  `status:running` for 10+ minutes across FOUR further user turns — its own
  conversation said "It already completed". `apply_subagent_update` derives
  `background_tasks` from the running-row count, but nothing ever closed a row
  whose completion frame never arrived (`mark_stopped` closes them only at session
  death). A dead child pinned the parent 'working' forever, so the parent never
  reached `ambientState: 'idle'`, **so its working→idle edge never fired and a
  dispatched fleet worker never reported finished.** Same class on the desktop:
  `SessionState::apply` zeroes `live_subagents` on SessionStart/UserPromptSubmit,
  but `hookEventRouter` did not mirror that onto its own `session.subagents`, and
  `applyStopEvent` keeps exactly the running rows while dropping completed ones —
  so a dropped SubagentStop leaves a phantom that survives forever.
  **FALSE-IDLE (signal vocabulary).** `claude_stream.rs`'s
  `background_tasks_changed` held the turn busy only for
  `task_type == "local_agent"`. The Claude Code 2.1.237 bundle's full vocabulary
  is `local_agent, in_process_teammate, remote_agent, local_bash,
  local_workflow` — `in_process_teammate` is the teammate/team feature (beside
  `leadAgentId`/`dynamicTeamContext`/`getConcurrentSubagents`) and `remote_agent`
  is cloud agents including `/code-review ultra`. Both are the parent waiting on
  another agent, both were classified ambient, so the parent's dispatch `result`
  idled it mid-subagent. (Also corrected: the old comment's claim that the CLI
  calls `local_workflow` "ambient/housekeeping" is not in the bundle — near it the
  bundle carries "Dynamic workflow", `workflow_agent`, `workflow_phase`,
  `agentControllers`. `local_workflow` was left ambient on a weaker, stated
  rationale.)
  **Turn boundaries are the reconciliation point** for parent/child state: a
  subagent row is scoped to the tool call that spawned it and cannot outlive its
  turn. Fixed in `SessionState::close_stale_subagents` (called from
  `set_managed_mode` when mode→Input, only after `write_pending` accepts, so a
  parked approval reconciles nothing) and `closeStaleSubagents` in
  `hookEventRouter` (SessionStart/UserPromptSubmit). Closing is the self-healing
  direction — the provider's next update re-opens the row. **When adding a
  busy-holding signal, enumerate the provider's whole vocabulary from its bundle
  rather than the one spelling you observed** (`strings` the CLI bundle and check
  what a literal sits beside).
- **`SubagentUpdate` is a sparse patch and it RECOMPUTES `background_tasks`,
  overriding the wire count.** Three behaviours of
  `SessionStore::apply_subagent_update` that are invisible from its signature:
  (1) every `Option` field is written only when `Some`, so there is **NO way to
  clear** a description/model/tool summary once set (only `status` is applied
  unconditionally; the store test asserts a Complete update with
  `description: None` still reads back the original). (2) It derives
  `background_tasks` as `subagents.iter().filter(status == Running).count()` on
  every call, which for a managed Codex session silently overrides whatever the
  wire/hooks reported — **the two writers do not merge**, each clobbers the other.
  (3) `completed_at` is set once via `get_or_insert` on the Running→Complete edge
  and cleared if a row goes Running again; timestamps are epoch **milliseconds**
  (`now_millis()`) while the surrounding `SessionState.updated_at` is an
  `OffsetDateTime` — do not mix the units. The stop path force-closes rows
  (every still-Running subagent flipped Complete with `completed_at` stamped,
  alongside the `background_tasks = 0` reset) or a dead Codex session would badge
  "working in background" forever. **Treat `background_tasks` as DERIVED for
  providers with `subagents[]` rows and as the wire count elsewhere**; if a
  provider ever needs both, add a separate field rather than teaching two writers
  to share one. To support clearing a field, make it tri-state — not a sentinel.

## Hand-authored notes (2026-08-28) — `isWakeTarget` (ex-`isSupervisor`) has a 5-file lockstep set

Renamed 2026-08-28. The flag is **not persisted anywhere** — no claudemon field
(a grep of `services/claudemon` is empty), no sqlite, no config.yaml, no boot
doc; the brain's `metaStore` and `claudeSessionStore` are both process memory —
but it IS a live hub-bus JSON key, so producers and consumers can only move
together. The complete set is FIVE files, and only four are findable by thinking
about "the wire":

1. `services/hub/cmd/brain/enrich.go` (`m["isWakeTarget"]` on `enrichSnapshot`)
2. `services/hub/cmd/brain/fleetview.go` (json tag)
3. `services/hub/cmd/hub/mobile.html` (`isManager` reads `s.isWakeTarget`)
4. `apps/desktop/src/main/services/hubCapabilities.ts` (`agents.list` row +
   `sessions.snapshots` spread)
5. **`apps/desktop/scripts/shootMobile.mjs`**, whose `session()` helper spreads
   the key straight into the snapshot `mobile.html` renders.

Miss the fifth and **nothing fails** — the staged mobile screenshots just quietly
lose the MANAGER chip and the crew nesting. `parity_test.go`'s
`nestingFieldsRequired` greps `mobile.html` for the literal field name, forcing
Go and mobile.html into one commit, but nothing at all guards the fixture.
**Treat `apps/desktop/scripts/shoot*.mjs` as first-class wire consumers alongside
`mobile.html`** — fixtures by directory, producers by behaviour, covered by no
test. Rust is NOT in the set (`apps/tui` and `services/claudemon` have zero
references) and the desktop renderer has zero functional references (two comments).

A partial rename degrades silently rather than failing: `/m` flattens the fleet
into an undifferentiated list with no manager chip, and the worker-finished wake
router stops recognising managers. The field named `isSupervisor` never meant a
role — its only input is `opts.manager`, which is why the rename was safe. Do NOT
confuse it with the three surviving namesakes: the `[supervisor]` blocked-wake
wire prefix in `main/shared/fleetMessages.ts` (parsed in three places incl.
mobile.html, round-trip tested), `app.supervisorHome`/`ensureSupervisorHome` (a
persisted IPC channel), and `services/hub/internal/supervisor` +
`services/hub/internal/nodes/supervisor.go` (OS process supervision). See
`modules/fleet-manager.md`.

## Hand-authored notes (2026-09-01) — the model string is a context-window carrier, and that is now bounded

Promoted from the 2026-08-30/31 model-window learnings, re-checked against
master at `0bac5799`. The learnings were written as a MIGRATION PROPOSAL; the
migration has since landed, so what follows is the shipped shape, not a plan.

- **`[1m]`/`-1m` is argv SYNTAX, not identity, and Claude Code strips it from
  the model id it writes into the transcript.** That is why the requested model
  was for a long time the only thing that knew a session was 1M, and why a
  session could publish a 200K denominator while genuinely holding a 1M window.
  A model string is therefore semantically load-bearing across spawn, restore
  and federation — not a display alias.
- **The canonical boundary now exists in one place per language and the marker
  is bounded ingress compatibility.** `contracts/model-context-windows.json`'s
  `selectionCases` block pins it: normalization always returns a base identity
  plus a numeric-or-null window, conflicts and empty identities are rejected,
  unknown identities survive, and feeding canonical output back through the
  normalizer is a no-op. `claudeArgvCases` pins the one place a marker is still
  EMITTED — external `--model` argv for the selectable Opus/Sonnet 1M variants;
  1M-native families (Fable/Mythos) and every unspecified selection stay
  undecorated. Implementations: `main/shared/modelContextWindows.ts` (+
  `canonicalSelection.ts` for the wire mapping),
  `services/claudemon/src/session/windows.rs`,
  `services/hub/internal/modelselection/modelselection.go`,
  `services/hub/cmd/brain/windows.go`.
- **The selection rides its own optional wire field now, additively.**
  claudemon publishes `requested_selection {model, context_window}` and
  `resolved_context_window`; the brain projects the camelCase pair beside the
  snake originals; the desktop, `/app`, `/m` and wks-tui all forward them
  without editing. Both spellings must be accepted on read — version skew
  across a federation link is normal, and an older brain sends only the snake
  form (the same way `status_line` and `tool_calls` still ride along unrenamed).
  Absence means "nobody has said" and must not be completed from a neighbouring
  field.
- **CORRECTION to the 2026-08-30 note "the requested Claude model is the only
  1M carrier":** it no longer is. Since `66c842df`/`73af1d02`/`4d4b8e9b` the
  canonical pair is published end to end, so a consumer that still re-derives a
  window from `settings.model` is manufacturing the second, disagreeing answer
  the slice exists to retire. `requested_model` survives as the compatibility
  projection only.
