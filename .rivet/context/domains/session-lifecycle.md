---
title: Session Lifecycle
tags: [sessions, state, snapshot, lifecycle, clear, manager, providers, subagent, liveness]
related_paths:
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/renderer/src/types/claudeSession.ts"
  - "apps/desktop/src/main/services/sessionStore/hookEventRouter.ts"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/store.rs"
  - "apps/desktop/src/main/services/sessionStore/pendingSlot.ts"
  - "apps/desktop/src/main/services/managerReplacementState.ts"
  - "apps/desktop/src/renderer/src/lib/sessionHistoryGroups.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Session Lifecycle

## State owners and projections

Claudemon’s `SessionStore` owns runtime session state and transport plumbing.
`SessionState` carries mode, pending decisions, provider/transport, usage evidence,
plan and subagent state. Claude PTY hooks drive its hook state machine; managed
adapters drive their session modes through the provider apply layer. Hooks may
enrich managed sessions without taking over their mode or pending decisions.

The desktop `claudeSessionStore` combines hooks, daemon state, conversation and
status-line observations into a richer snapshot. The full-scope brain maintains
its own projection for headless clients. Neither projection is an identical copy
of the daemon model. Renderer `status`/`ambientState` and daemon `SessionMode`
are different vocabularies; use their existing mapping functions.

The desktop store’s `ClaudeSessionSnapshot = Omit<ClaudeSessionState, never>`
omits no fields. Object spreading is not field redaction. Snapshot getters
explicitly detach pending payloads; publication uses a compact background
snapshot plus full detail for watched sessions. See
[the IPC boundary](../modules/ipc-boundary.md) and
[the backend seam](renderer-backend-seam.md).

## Spawn, resume, and teardown ownership

Launchers pre-register a pinned session ID and metadata before runtime events
arrive. The daemon also uses execution-engine admission and durable generation
identity; see [agent spawning](agent-spawn.md).

A resume can reuse the same session ID. `register_spawn` revives a stopped row
to Unknown and clears prior pending state before the new hook arrives. A late
teardown from the old process must not remove its successor:

- `claim_generation`/`owns_generation` fence release, failed-spawn cleanup, and
  managed deregistration. An unclaimed ID is treated as owned for cleanup;
  an existing mismatched generation is not.
- `reap_pty_owned` compares the PTY handle identity before removing its registry
  slot, while still reaping the caller’s own child.
- Desktop eviction timers are cancelled by restart/pre-registration signals and
  re-check that the session is still ended before evicting it.

`release_spawn` removes live plumbing but retains a stopped state row for
history. `drop_pending_spawn` removes the failed setup’s row and aliases pointing
at it. Shared cleanup removes a cwd’s pending slot only if it still names that
session. Aliases are therefore not uniformly permanent; preserve the difference
between failure cleanup and retaining a previously used session.

`claudemonSessionClient.verifyAttachTarget` reports a stopped target as exited
but keeps the viewer stream, allowing a later life of the same ID to reappear.
A 404 tears the viewer down. `exitNotified` prevents duplicate exit banners.
A stopped retained row, an absent row, and an unreachable daemon are distinct.

## Message delivery

`submit_message` accepts live-session messages immediately or queues them during
cold start, active work, or a pending dialog. Stopped sessions reject; missing
plumbing, disconnected wrappers, and queue exhaustion have separate responses.
The PTY settle/verify pipeline uses `FLUSH_DELAY_MS = 300`, `input_since`, and
`client_input_at` to avoid submitting into a redrawing composer. Do not apply
that PTY timing rule as though every managed adapter types terminal bytes.
See [the HTTP API](claudemon-http-api.md#message-and-action-responses).

A transcript interruption marker can supply a stop transition when no Stop hook
arrives. It still goes through the hook-owned state path rather than clearing a
managed/federated decision unconditionally.

## Pending decisions: feed ownership and request ownership

The desktop’s `sessionStore/pendingSlot.ts` selects one feed for a row:

| Row | Feed allowed to park/resolve it |
| --- | --- |
| Any row carrying `hub` | Federation, checked before provider/transport |
| Local Claude PTY (including legacy absent transport) | Hooks |
| Other local managed/stream row | Daemon |

Use `PendingSlot` for mutations and `bornWithPending`/`bornWithEmptyPending` for
construction. A readonly property does not prevent an object literal from
initializing it, which is why construction has its own `SessionWithoutPending`
type. Successful answer acknowledgement clears questions through its explicit
path; it is not a general permission to clear approval cards.

`PendingFencedSession` constrains store-local assignments. Collaborators use
`PendingReadOnlySession`, including readonly question arrays/options; merely
making the outer array property readonly would still allow pushes/splices.
These types are compile-time constraints, not runtime freezing. Snapshot copying
protects normal JSON payloads from caller mutation. `detachToolInput` uses
`structuredClone`, but returns the original object if cloning fails; do not
claim unconditional deep isolation for arbitrary non-JSON objects.

Inside claudemon, feed ownership does not imply only one outstanding request.
`PendingOwner::Primary` represents the hook/driver path and `Ask` the MCP question
shim. `PendingWrite::Park` can displace the other owner’s card; resolving one
owner restores an outstanding displaced card instead of clearing both. The
private pending field forces writes through `write_pending`. Callers use the
state returned by `set_managed_mode`, because their requested mode may not be
the mode left after ownership arbitration. `QuestionGuard` also releases its
owned question if its request is aborted.

## Subagents and background work

Hook `PreToolUse` is deduplicated by tool-use ID across active/completed calls.
Subagent tool calls are excluded from the parent’s work-log cards, but supported
Edit/MultiEdit/Write changes still enter the parent file-change list.

Claude hook bookkeeping uses `live_subagents` and `parent_turn_ended`. A parent
Stop while subagents remain keeps an unblocked session responding; an existing
Approval/Question mode is preserved. The final SubagentStop can return it to
Input. These counters are non-serialized and reset at session/turn boundaries.

Managed provider subagents are keyed by their provider-native child identity.
`apply_subagent_update` applies present optional fields and always applies status;
None means keep the old optional value, not clear it. It re-derives
`background_tasks` from running subagent rows, which can replace a separately
reported wire count. Do not add another counter writer without defining how the
sources combine. `completed_at` is set once on completion and cleared on Running;
these subagent timestamps are epoch milliseconds, unlike `updated_at`’s time type.

`close_stale_subagents` closes remaining running rows when an accepted managed
transition reaches Input. A blocked pending write does not run that reconciliation.
Desktop `closeStaleSubagents` reconciles on SessionStart/UserPromptSubmit. A
subsequent provider update can reopen a row. Providers with no subagent rows do
not have their background shell count re-derived by this cleanup.

The Claude stream driver distinguishes agent work from ambient shell/workflow
activity. Keep its task-kind classification and busy/idle behavior together;
“some background task exists” is not sufficient proof the parent is responding.

## Liveness is not token activity

Only Claude PTY has the heartbeat-like native status-line command that the
renderer stall detector treats as periodic evidence. Managed/stream status lines
are activity-driven; their timestamps can stop advancing while the process is
still alive. `stallDetector` therefore reports alive, silent, or unknown rather
than converting all old status lines into dead sessions.

Managed teardown publishes stopped state. The daemon’s ghost sweep also uses
runtime plumbing and idle thresholds; it does not reap a session merely because
no token update arrived. Do not substitute context occupancy, cumulative usage,
or a progress fingerprint for process liveness.

The daemon `/events` stream emits `session.resync` after broadcast lag. Desktop
and brain consumers reconcile authoritative state on reconnect/resync because a
lost terminal event may never have a later update to repair it. Other clients
must implement that reconciliation explicitly; receiving SSE is not itself a
replay guarantee.

## Retention and resumable history

Desktop ended rows normally have a 30-second eviction grace period. Daemon
`is_archived` hides stopped rows older than seven days; maintenance evicts those
rows from memory and prunes old SQLite session/event rows beyond the newest-100
retention floor. Database pruning uses last-event age, not the in-memory mode.
Provider transcript retention is a separate concern. See
[SQLite persistence](../modules/claudemon-sqlite-store.md).

The History pane merges per-project Claude transcript listings with daemon
resumable rows through `sessionHistoryGroups.ts`. A transcript-only session can
appear after the daemon has forgotten it; analytics `session_history` is not the
source of resumability. A matching daemon row preserves recorded provider,
transport, and model. Open/live IDs are excluded to avoid offering a second
launch of an already-running session. Daemon-row title enrichment is capped at
40; the pane’s list is not capped by that title lookup limit.

## Manager lineage and restart recovery

Manager metadata is not a claudemon session-row field. It is nevertheless durable
for eligible launches through `managerReplacementState.ts` and its private
`manager-replacements.json` journal. Recovery restores label, parentSessionId,
isWakeTarget, provider/transport, settings and routing attribution, then
reconciles replacement operations. Desktop overlays reject remote rows; the
headless companion uses the shared recovery service.

The journal restores attribution, **not process liveness**. Live tombstone/orphan
maps remain process-local projections. A missing live manager or dangling parent
ID alone is not authority to adopt workers. Use the replacement record and owning
hub, and keep uncertain delivery acknowledgements uncertain rather than blindly
replaying a mutating operation.

`isWakeTarget` is a wire and journal field. Changes must update current producers,
readers and fixtures, including mobile screenshot fixtures. It is unrelated to
OS process-supervisor types or the historical supervisor message prefix.

## Model selection across lifetimes

The canonical selection is an identity plus optional context window, distinct
from runtime-reported capacity and observed occupancy. Legacy `[1m]`/`-1m`
spellings are normalized at ingress and emitted only where provider argv requires
them. Optional `requested_selection`/`resolved_context_window` evidence is
preserved through persistence, transport projections and federation; absence is
not filled from a nearby field that happens to look plausible. The cross-language
contract is `contracts/model-context-windows.json`; see
[usage accounting](usage-accounting.md) for claim precedence and freshness.

## Verification

Relevant suites cover daemon state/store ownership, pending-slot boundaries,
renderer history grouping and stall signals. Use existing generation/restart and
pending-owner regression tests when changing these paths, plus the API tests for
queue/response behavior. Model/context changes additionally require the shared
contract loaders, and manager recovery changes require the companion integration
suite. Unit coverage does not prove every live external provider version behaves
identically.
