---
title: Transcript paging bounds DOM but not full-history derivation
date: 2026-09-23
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/renderer/src/lib/anchorWork.ts
promoted: false
---

# Transcript paging bounds DOM but not full-history derivation

## Observation
ClaudePane defaults CONVERSATION_PAGE_SIZE to 60 and renderedConversation slices at :2259, while each active snapshot update still counts user sends across the retained conversation at :1570, calls anchorWork over all turns at :2148-2150, and rebuilds the complete transcript tool-id Set at :2163-2175. anchorWork.ts:40 scans all turns and workflow matching at :68-75 searches prior workflow calls per completed workflow. Fresh IPC conversation arrays invalidate these useMemo dependencies even when old turns are unchanged. Hidden panes are mitigated by 12-turn compaction and 1s flushes; active panes retain full history. ConversationMessage React.memo has only default shallow comparison, so fresh cloned turn props bypass it, although content-based markdown memoization protects unchanged parsing.

## Impact
Visible chat rendering cost still grows with retained transcript/tool history despite showing only the most recent page; paging is not an end-to-end CPU bound.

## Recommendation
Maintain incremental user-count/tool-id/anchor indices keyed by session and conversation revision/offset, and preserve completed turn identities at the renderer store boundary; reset correctly on truncation/restart.
