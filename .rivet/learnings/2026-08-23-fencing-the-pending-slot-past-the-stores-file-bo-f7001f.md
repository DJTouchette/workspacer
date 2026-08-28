---
title: Fencing the pending slot past the store's file boundary needs `readonly T[]`, not `readonly` — and losing assignability back to ClaudeSessionState is the feature
date: 2026-08-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/sessionStore/pendingSlot.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/pendingSlotBoundary.test.ts
  - apps/desktop/src/main/services/sessionStore/conversationApplier.ts
promoted: false
---

# Fencing the pending slot past the store's file boundary needs `readonly T[]`, not `readonly` — and losing assignability back to ClaudeSessionState is the feature

## Observation
Census of everything OUTSIDE claudeSessionStore.ts that is handed a LIVE session row (via `this.sessions.values()`/`.get()`), completing the earlier four-writer census with the READER side: (1) agentNotifier.notifyOnTransition, (2) supervisorNudge.onBlock/broadcastBlock/onFinished, (3) SessionUsageAccumulator.applyUsage + static refreshContextLimit, (4) conversationApplier (4 entry points), (5) budgetWatcher.checkBudget, (6) analyticsWriter.writeHistory. Two were named in the dispatch; six exist. Everything else that looked like an exit is either internal to the class body (supervisorSessionIds, reparent, orphanCandidates, pruneManagerTombstones, markHubPeerOffline, reconcile) or already a structural Pick that excludes the slot (supervisorNudge's FinishedWorker, thresholdWatch, progressReports, workerFailure).

Two facts the earlier `PendingFencedSession` work did not have to confront:

(a) `readonly pendingQuestions: PendingQuestion[]` stops `session.pendingQuestions = null` (TS2540) and NOTHING else. `.push()`, `.splice()`, `.length = 0` and `[0] = …` all reach the store's own array. Stopping those needs `readonly ReadOnlyPendingQuestion[]`, and the payloads need freezing too or `card.toolInput.command = …` still rewrites the live card. The new `PendingReadOnlySession` in sessionStore/pendingSlot.ts does both. Verified errors: TS2540 on assignment, TS2339 `Property 'push' does not exist on type 'readonly ReadOnlyPendingQuestion[]'`, TS2551 for splice ("Did you mean 'slice'?"), TS2540 on `pendingApproval.toolName`.

(b) The cost is that `PendingReadOnlySession` is NOT assignable back to `ClaudeSessionState` (a `readonly T[]` is not a `T[]`), which is the exact property the previous worker relied on to fence the store with zero call-site churn. That turns out to be desirable — a fenced collaborator cannot launder the row by passing it to something mutable — but it means the fence must be widened in exactly ONE place instead: `PendingSlot`'s constructor now takes `PendingReadOnlySession` and casts internally. That was forced by a real case, not hypothetical: conversationApplier is NOT a pure reader — its interrupt path calls hookEventRouter's `applyStopEvent`, which legitimately clears the slot through a gated `PendingSlot('hooks')`. Widening the constructor let every fenced module keep its legitimate gated write.

Separately, `getSnapshot`/`getAllSnapshots` shallow-cloned, and the alias was live, not theoretical: `snap.pendingApproval.toolName = 'Read'` on a snapshot changed the store's card (test caught it as `expected 'Read' to be 'Bash'`), and `snap.pendingQuestions.length = 0` unblocked a session that was still blocked. Fixed by COPYING (detachPendingSlot) rather than typing: `ClaudeSessionSnapshot` is declared THREE times (main/services/claudeSessionStore.ts as `Omit<ClaudeSessionState, never>`, main/shared/ipcTypes.ts, renderer/src/types/claudeSession.ts), so a readonly type would have had to land in all three to mean anything and would still only bind code that opts into the type — whereas a caller mutating a detached copy simply cannot reach the store, cast or no cast.</observation>
<parameter name="impact">The pending-slot invariant is behind every worker-freeze this project has shipped. Before this, a `.push()` or a `.length = 0` on a snapshot's question array — no assignment, so no `readonly` catches it, no `PendingSlot`, so no ownership check runs — reached the store's live slot. Anyone who "hardens" a slot with a bare `readonly` modifier and stops there has fenced only half the writes.

## Recommendation
A collaborator outside the store takes `PendingReadOnlySession`, never `ClaudeSessionState`. A legitimate write goes through `PendingSlot`, whose constructor is the single widening point. Anything leaving the store as a snapshot goes through `detachPendingSlot` (getSnapshot, getAllSnapshots, the publishSnapshot factory); `webContents.send` does not need it because Electron structured-clones. Guards live in apps/desktop/src/main/services/pendingSlotBoundary.test.ts — the snapshot cases fail outright without the copy, the collaborator cases fail against a mutant collaborator.
