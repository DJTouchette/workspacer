---
title: Codex subagent drill-in now has TWO item-folders and TWO identical text formatters that can drift apart
date: 2026-08-26
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/providerSubagentConversation.ts
  - apps/desktop/src/renderer/src/backend/busConversation.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/main/services/sessionStore/conversationApplier.ts
promoted: true
promoted_to: renderer-backend-seam
---

# Codex subagent drill-in now has TWO item-folders and TWO identical text formatters that can drift apart

## Observation
Because the same Codex child rollout is rendered by both the desktop-local path and the web/bridged path, bcef811b + 7f4cf276 left four pieces that must stay in agreement, none of which reference each other:

- **Two `applyConversationItems`.** `main/services/sessionStore/conversationApplier.ts:194` and `renderer/src/backend/busConversation.ts:223` are separate implementations with the same name. The main path folds through the first, the web path through the second.
- **Two one-shot folders on top of them.** `providerSubagentConversation.ts` `foldItemsToConversation` vs `busConversation.ts` `foldConversationItemsToTurns`.
- **Two byte-identical transcript formatters.** `rawText` (main) and `transcriptLineText` (webBackend) — same `⚙ `/`↳ ` markers, same `slice(0, 400)` truncation. Verified identical modulo the function name. Change one and the same subagent reads differently in the desktop than in the browser.

The main-side folder is the trap. It fabricates a throwaway session object and casts it: `applyConversationItems(temp as Parameters<typeof applyConversationItems>[0], items, () => {})`. That cast is what lets a session-shaped literal with dummy `ptyId`/`transcriptPath: ''`/hardcoded `provider: 'codex'` satisfy the applier — so adding a REQUIRED field to the applier's session type will NOT produce a type error here; it will produce `undefined` at runtime. The renderer side has no such problem because it uses the real `newConversationState()`.

Also on the web path only: rollout items carry no timestamp, so `webBackend` stamps every folded turn with one shared `Date.now()` fallback. Turn ordering therefore comes from item order, not from timestamps — a consumer that sorts by timestamp gets an arbitrary order.</observation>
<parameter name="impact">Silent desktop/web divergence in how a Codex subagent transcript reads, plus a type-checked-looking cast that is not actually checked.

## Recommendation
When touching subagent transcript rendering, grep for BOTH formatters and BOTH folders and change them together. If either is edited substantively, prefer promoting the formatter into `main/shared/` (the existing home for cross-process twins like `mergeConversationWindow`) rather than maintaining a third copy.</recommendation>
<parameter name="confidence">high
