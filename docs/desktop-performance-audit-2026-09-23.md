# Desktop performance audit — 2026-09-23

Scope: desktop responsiveness with many agents, long conversations, and slow
agent startup. Reviewed `master` at `f96d0010` plus the current working-tree
history batching and startup timing changes. Three independent reviewers
covered renderer, backend/event delivery, and history correctness; the main
review covered startup and consolidated their findings.

The findings below are confirmed code paths. Their ranking is an engineering
assessment, not a CPU profile of the affected desktop. Agent count, provider
mix, machine specifications, and local/remote topology remain unknown.
No further production changes were made during the audit itself. The subsequent
authorized implementation is summarized below; the findings retain their
original line references as an audit baseline.

## Implementation follow-up

| Finding | Implemented response |
|---|---|
| 1 — IPC payloads | Compact global snapshots; reference-counted active detail stream; compact hidden reads; reload cleanup. |
| 2 — fleet state | 100 ms fleet-wide routine batching with immediate lifecycle/decision updates and stable unchanged status map. |
| 3 — context fanout | Separate stable action and indexed agent-directory contexts; shared per-agent decision index. |
| 4 — transcript work | Safe unchanged-turn reconciliation plus incremental user/tool/orchestration indexes. |
| 5 — tail scheduling | Bounded reads, eight concurrent session passes, cached subagent discovery and idle checks; backlog draining preserved. |
| 6 — worktree discovery | Git HEAD directory candidates; shared in-flight lookup and 30-second cache; source revalidation and explicit refresh. |
| 7 — model probes | Shared concurrent misses, bounded optional bundled-model probe and cleanup, desktop request timeout. |
| 8 — history stalls/logging | Common transaction timing records lock wait and write cost for all triggers; critical lifecycle durability preserved. |
| 9 — inspector reads | Coherent tasks/requests read; shared renderer polling; headless metric refresh batches writes. |
| 10 — dropped state events | Explicit lag marker and authoritative lifecycle reconciliation with race protection. |

See [runtime behavior and logging](desktop-agent-performance.md) for cadences
and remaining limits. Full active IPC history and critical synchronous history
transactions remain; no claim of zero main-thread stalls or measured desktop
frame-rate improvement is made.

Implementation validation: full main-process suite passed 3,676 tests; the
renderer run passed 1,922 tests and exposed five outdated mock-fixture failures,
all corrected and verified in affected reruns. Subsequent renderer race tests
also pass. Full Rust library passed 880 tests (four ignored); final focused
conversation/API additions, desktop SSE/bridge recovery, and Go reconnect/lag
tests pass. TypeScript checks, headless JSON-protocol integration and the renderer
production build pass. No real multi-provider desktop fleet latency trace was
captured in this environment.

## Findings in recommended order

### 1. Full session histories still cross desktop IPC on every flush — high priority

`apps/desktop/src/main/services/claudeSessionStore.ts:2567` and `:2589` send
`{ ...session }` over `claude-session:update`. Coalescing is per session at
16 ms; there is no visibility-based payload selection. Renderer compaction
happens after the transport cost has already been paid. Hub publication does
compact before sending (`services/hubTelemetry.ts:37` in the same main tree).

Long conversations multiply structured-clone, allocation and garbage-collection
cost by update frequency and active agent count. The synthetic benchmark's
2,000-turn fixture is about 4.07 MB JSON versus 24.6 KB compact; JSON size is a
payload proxy, not the measured byte size or latency of Electron IPC.

Recommended change: compact global/background updates at the producer, with a
separate active-transcript delta/subscription path. Do not simply truncate the
existing shared update: active panes rely on it for complete history.

Verify with payload sizes, IPC message rates, and main/renderer CPU at 1, 10,
and 30 sessions, with short and long transcripts. Test full-history restoration,
active-pane switching, reconnects, approvals, and conversation offsets.

### 2. Every snapshot invalidates global fleet state — high priority

`apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts:90` replaces both
global maps even when ambient state is unchanged. `useAttentionFeed.ts:209`
recomputes all-session fingerprints, `:279` rebuilds attention across agents,
and `:232` can schedule another update when progress fingerprints change.

Work scales with fleet size times incoming update rate. When each agent emits
independent updates, total work can approach quadratic scaling; React batching
can combine coincident updates, so one event does not necessarily mean one render.

Recommended change: batch routine snapshot promotion, preserve unchanged status
identity, and derive attention incrementally per session. Keep decision and
lifecycle transitions prompt. Validate with React commit duration/count and
typing/scroll latency during concurrent streaming.

### 3. A shared attention context defeats hidden-workspace memoization — high priority

`apps/desktop/src/renderer/src/contexts/AttentionContext.tsx:323` combines
volatile snapshots/feed with actions in one context and publishes it at `:355`.
Memoizing the action object does not isolate consumers of that single context.
`components/claude/FleetMessageCard.tsx:108` subscribes every session chip;
`:109` searches the agent array. Hidden manager transcripts can retain many
such chips. `components/AgentCard.tsx:137` also consumes this context.

Recommended change: split actions and stable agent membership into separate
contexts, or use selector subscriptions; index per-agent lookups. Existing
`AgentWorkspaceView` memoization does not stop nested context subscribers.
Validate that streaming agent A does not rerender unrelated hidden chips/cards.

### 4. Transcript pagination does not bound active-pane computation — medium priority

`apps/desktop/src/renderer/src/panes/ClaudePane.tsx:150` starts with 60 rendered
turns, but `:1570` counts user sends over retained history, `:2148` anchors work,
and `:2163` rebuilds tool-ID membership from the full conversation.
`lib/anchorWork.ts:40` scans history, with further workflow searches at `:69`.
Fresh IPC identities invalidate these memoized derivations.

Recommended change: incremental counters, tool indexes and work anchors, with
explicit reset/offset semantics and stable identities for completed turns.
Hidden-pane compaction and markdown content memoization already help; this is
specifically residual active-pane work. Profile equal streaming rates at
60, 500 and 2,000 turns before changing the rendering architecture.

### 5. One large transcript backlog delays other tailed sessions — medium priority

`services/claudemon/src/session/conversation.rs:378` runs a single 400 ms tail
loop, awaiting each session and its subagents serially (`:400`, `:403`).
`tail_one` reads the whole newly available backlog (`:462`) and parses it before
moving on. Subagent handling rescans/sorts directory membership (`:567`) and
stats every historical JSONL file (`:584`) even when content is unchanged.

Recommended change: bounded reads per session with fair scheduling or bounded
concurrency, and cached/watched subagent membership. Preserve byte cursors,
partial lines, usage deduplication and ordering. This affects transcript-backed
sessions; do not assume every provider's native streaming adapter uses this loop.
Verify by resuming a large transcript while a second small transcript advances,
then measure that second session's delivery latency and filesystem operation count.

### 6. Worktree creation repeatedly walks the entire visible source tree — medium priority

`apps/desktop/src/main/services/worktreeService.ts:158` walks every non-dot,
non-`node_modules` directory serially, then `:166` checks for `node_modules`
under each. It does not prune ignored build trees such as `target`, `dist`, or
`vendor`. Every `linkNodeModules` call repeats discovery (`:190`), and creation
awaits it before setup/provider launch. This is relevant only to worktree-backed
spawns; configured setup commands can add their own expected delay.

Recommended change: coalesce/cache discovery per canonical source repository,
with explicit refresh and path validation; consider workspace-manifest discovery
or pruning build outputs. Preserve clean-tree and symlink checks. Measure the
new `worktree_dependency_links` stage against repositories with large build trees.

### 7. Catalog cache misses can spawn duplicate probes; one Codex probe is unbounded — medium priority

`services/claudemon/src/providers/mod.rs:131` caches completed model lists for
600 seconds but does not share an in-flight request. Concurrent cold/expired
requests each await their own subprocess-producing fetch. Codex then runs
`debug models --bundled` at `providers/codex.rs:917` outside the preceding
10-second app-server read timeout, without a deadline or kill-on-drop on this
second process. Electron's `claudemonSessionClient.ts:310` also fetches without
an AbortSignal.

Recommended change: per-key in-flight deduplication, bounded probe lifetimes
with child cleanup, and a client deadline. Test simultaneous misses with a fake
provider and a probe that hangs. Composer menus load lazily; **not every mounted
pane starts a model probe**. Settings, spawn dialogs and separate clients can
still issue concurrent requests. This is catalog/picker startup overhead, not
an unconditional step in every agent spawn.

### 8. Lifecycle persistence can still stall, and its timing is not logged — medium priority

The first fix deliberately preserves immediate durability for lifecycle changes.
`apps/desktop/src/main/services/dispatchHistoryStore.ts:90` flushes the pending
fleet on transitions and ended observations; the transaction at `:119` still
reloads, clones, compares and writes synchronously. Lock contention can wait
250 ms (`main/lib/configLock.ts:49`, synchronous `Atomics.wait` in `fileLock.ts`).
Only the timer callback at `dispatchHistoryStore.ts:103` records flush duration.
Lifecycle, admission and shutdown writes therefore escape that timing signal.

Recommended change: instrument the actual transaction and lock acquisition for
all triggers first. Measure simultaneous start/idle/end bursts and contention
before deciding whether an asynchronous persistence owner is warranted. Keep
critical durability and cross-process update ordering intact.

### 9. Task Inspector polling rereads the same history once per manager — medium priority

`apps/desktop/src/main/ipc.ts:1501` calls `listRequests` per manager, then
`listForHostUser`; each invalidates the cached history and reads/parses the file.
Task Inspector and Recent Agents independently poll at three-second intervals.
With M managers and P mounted polling consumers this produces roughly
P × (M + 1) history parses per polling round, before other callers.

Recommended change: one coherent tasks-and-requests read per response and shared
renderer polling. Validate the filesystem-read count with several managers and
both consumers open. These reads do not themselves flush the pending metrics.

### 10. Event overload can lose a session's final state — conditional correctness issue

`services/claudemon/src/session/store.rs:22` bounds state-event broadcast storage
at 256 entries. If a subscriber falls behind, `daemon/api.rs:1556` logs the lag
and drops that result without sending a resync marker. The desktop's
`apps/desktop/src/main/services/claudemonEventBridge.ts:64` depends on SessionEnd
to end managed sessions; this stream has no sequence-gap reconciliation.

A terminal event lost during overload can therefore leave a managed card looking
live and retain its state. This is a source-confirmed conditional failure path,
not a reproduced incident on the user's machine. Unlike conversation deltas,
the state stream does not offer the same gap-recovery mechanism.

Recommended change: explicit lag/resync signaling and an authoritative session
refresh, also on reconnect. Include an overflow test that drops a terminal event
in the next transport work; reducing payload volume alone does not guarantee
recovery. Treat this as a correctness companion to findings 1–3.

## Scope-dependent follow-ups

- Headless `apps/desktop/src/main/headless/desktopHost.ts:683` still compares
  complete tracked snapshots and synchronously observes each changed session.
  The desktop batching change does not improve that host path.
- Codex creates a dedicated app-server per managed session. That is real resource
  scaling, but process sharing is an architectural change with isolation and
  lifecycle implications, not an automatic optimization.
- Spawn-time skill validation, PATH lookup, and token-file writes remain
  synchronous. Use the new preflight/launch-assets timings to determine whether
  they matter on the affected machine before caching or moving them.
- The headless conversation relay already has demand gating. An older context
  description of universal 500 ms conversation polling is stale and is not a
  current finding.

## Validation and limitations

- Independent history review found no confirmed correctness regression in the
  traced desktop admission/shutdown paths. Its focused history, task-inspector
  and workflow-store run passed 39 tests.
- Repeated production-code synthetic benchmark: 200 stored tasks and 120 steady
  updates took 472.79 ms with immediate observation versus 4.75 ms batched;
  fsync count fell from 120 to 1. Includes final commit; excludes lifecycle
  bursts and large populated request-history stress.
- Existing Go bus benchmarks passed: 100-iteration status-only round trips
  averaged about 0.118 ms versus 0.936 ms for the synthetic 200-turn payload.
  This supports payload cost, not a claim that the hub dominates desktop latency.
- No real provider fleet, desktop CPU profile, memory trace or input-latency
  recording was collected. Remaining impact estimates require those measurements.

Recommended next implementation slice: compact/selective desktop delivery plus
batched fleet promotion and context isolation, with transcript/approval regression
tests. In parallel with measurement, complete transaction-level timing. Address
worktree discovery and catalog probe deduplication as separate startup changes.
