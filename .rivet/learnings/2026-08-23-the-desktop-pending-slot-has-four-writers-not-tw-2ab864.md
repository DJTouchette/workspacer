---
title: The desktop pending slot has FOUR writers, not two — and a peer-mirrored row is nobody local's
date: 2026-08-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/sessionStore/pendingSlot.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/sessionStore/hookEventRouter.ts
promoted: false
---

# The desktop pending slot has FOUR writers, not two — and a peer-mirrored row is nobody local's

## Observation
Census of every writer to `pendingApproval`/`pendingQuestions` on a live session row in apps/desktop/src/main: (1) the hook feed (hookEventRouter), (2) the daemon feed (claudeSessionStore.applyManagedPending), (3) the FEDERATION feed — `upsertRemoteSession` + `upsertSparseRemoteSession`/`sparsePendingApproval`, which rebuild the whole row from a peer's wire snapshot, and (4) `clearPendingQuestions` (ipc.ts CLAUDE_ANSWER, hubCapabilities claude.answer), an optimistic clear nobody had named. (3) and (4) were unlisted in every prior dispatch.

Ownership used to be computed from provider/transport alone (`provider !== 'claude' || transport === 'stream'`), which says 'hooks' for a peer's claude/PTY row. So both LOCAL feeds could park or resolve a card on a row that is only a MIRROR of a request parked on another machine. `upsertRemoteSession` already refuses the reverse direction (a peer overwriting a local row) with a console.warn; the local-over-peer half was missing. `pendingSlotOwner` now checks `session.hub` first and returns 'federation'.

(4) is genuinely legitimate cross-feed and needed a word the hook feed's Park/Resolve vocabulary does not have: the user answered and the OWNER accepted the answer, so the clear resolves the exact request that feed parked rather than guessing about it. It is exported as a free function `acknowledgeAnswer()` (no feed argument, ungated) and narrowed to questions only — an approval goes through `claude.approve`, whose decision the owner may still reject as unknown, so optimistically clearing that card would hide a still-open request.

Two TypeScript facts that shape the fence: `readonly` is invisible to assignability, so a fenced session still passes anywhere a `ClaudeSessionState` is wanted (notifier, watcher, publishSnapshot) — typing `private sessions = new Map<string, PendingFencedSession>()` fences the ENTIRE store class body in one line with zero call-site churn. But `readonly` only forbids assignment AFTER initialization: an object literal may still name the field, which is exactly how the federation feeds write (they rebuild the row wholesale). The construction-side fence is a separate type, `SessionWithoutPending = Omit<ClaudeSessionState, 'pendingApproval'|'pendingQuestions'>` — naming either field in a literal of that type is TS2353. Verified both: TS2540 on assignment, TS2353 on construction.</observation>
<parameter name="impact">Every worker-freeze this project has shipped is a violation of "exactly one feed owns the pending slot". The federation hole meant a local claudemon frame carrying a remote session's id could empty a peer's card (blocked state, nothing to answer) or park one whose Approve button posts to a daemon that never heard of the request. The `Keep`-the-timestamp rule (an unchanged re-sent card must keep its original stamp or the needs-you dock resurrects dismissed cards) was hand-rolled independently in applyManagedPending and sparsePendingApproval and MISSING entirely from the full-remote path.

## Recommendation
All four feeds now go through apps/desktop/src/main/services/sessionStore/pendingSlot.ts — `pendingSlotOwner` (one function, so hook feed and store cannot drift), `PendingSlot` (park/resolve, gated on the declared feed, each returning what the slot HOLDS), `acknowledgeAnswer` (ungated, questions only), `bornWithPending`/`bornWithEmptyPending` (construction). A new writer that forgets the check does not compile. Note the fence is file-local: `sessions.values()` still hands live rows to supervisorNudge and the notifier, which only read the slot today — if one ever writes it, fence there too.
