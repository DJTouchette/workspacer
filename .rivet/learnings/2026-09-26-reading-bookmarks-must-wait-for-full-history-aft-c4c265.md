---
title: Reading bookmarks must wait for full history after agent activation
date: 2026-09-26
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/hooks/useChatReadingPosition.ts
  - apps/desktop/src/renderer/src/hooks/useClaudeSession.ts
  - apps/desktop/tests/e2e/chatTailPin.test.ts
promoted: false
---

# Reading bookmarks must wait for full history after agent activation

## Observation
Inactive useClaudeSession snapshots are compacted to a short window. Restoring a chat bookmark immediately when isActive flips can therefore discard a valid older anchor or mistake truncated reply text for new content. The renderer now exposes activation-scoped detailReady and useChatReadingPosition waits for that fetch before restoring. Bookmark state is localStorage-backed, bounded to 128 sessions and 30 days, and contains coordinates and fingerprints rather than transcript text. Browser coverage verifies reload, compact-to-full switching, pagination, streaming growth and unfocused arrivals.

## Impact
Reading continuity belongs to the viewer, and background snapshot receipt must never count as user reading. Existing global conversation indices remain the anchor coordinate space.

## Recommendation
Keep full-history readiness, page/window visibility and tail-follow behavior aligned when changing chat lifetimes. Preserve the chatTailPin browser checks because jsdom cannot verify scroll geometry.
