---
title: The web bus conversation fold must be copy-on-write — React.memo eats in-place turn mutation
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/busConversation.ts
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/renderer/src/components/claude/ConversationMessage.tsx
promoted: false
---

# The web bus conversation fold must be copy-on-write — React.memo eats in-place turn mutation

## Observation
ClaudePane memoizes on the conversation ARRAY's identity (`useMemo(...,
[session?.conversation])`) and `ConversationMessage` is `React.memo`'d on the
TURN object. The desktop never notices because every snapshot crosses Electron
IPC and arrives as a fresh structured clone. The web seam (`busConversation.ts`)
hands the renderer the very objects it holds, so an in-place
`last.content += fragment` is invisible: measured live (real `workspacer serve`
+ real /app bundle in headless Chromium, daemon-truth vs a 33 ms DOM sampler),
the transcript DOM froze on the FIRST fragment for an entire 18-second turn
while the fold state underneath was perfectly current — the state-edge poke's
`sinceSeq` kept marching, proving the data plane was fine and only rendering
was dead. 31d9d4cb independently hit the array half (frozen `lastUserTs` memo)
and fixed `merge()` to slice; the turn-object half still froze every grown
bubble until the fold went copy-on-write (conversation-delta-push merge).

## Impact
Any renderer-side fold that feeds `onClaudeSessionUpdate` (or any snapshot
path) must replace, not mutate, the objects React memoizes on — content-level
assertions in unit tests all pass while the real UI freezes, so only a
DOM-clock measurement (or an identity assertion) catches it.

## Recommendation
Keep busConversation copy-on-write: a changed fold replaces the array AND
exactly the changed turn objects (untouched turns keep their memo identity —
that is the perf point of the memo). The identity contract is pinned by
"gives a changed fold fresh array and turn identities" in
tests/backend/busConversation.test.ts; new mutation sites in the fold must
follow it.
