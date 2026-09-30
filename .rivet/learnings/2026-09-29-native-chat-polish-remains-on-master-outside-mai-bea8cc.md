---
title: Native chat polish remains on master outside main after branch transition
date: 2026-09-29
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/**
promoted: false
---

# Native chat polish remains on master outside main after branch transition

## Observation
As of 2026-09-29 after fetching origin, local master and origin/master point to 0c89afff (Polish native chat, tool cards, and session controls), which is not an ancestor or patch-equivalent commit of origin/main (1bf2f53a). master...origin/main is 1 versus 26 commits. The master commit adds timing.rs, tool_preview.rs, ui/scroll.rs and ui/tools.rs; these paths are absent from origin/main, which has a separate transcript implementation. GitHub HEAD points to main, while this checkout's cached origin/HEAD still points to master. All eight local stashes and all existing registered worktrees were checked; no stashed or dirty apps/native work was found.

## Impact
Native chat polish is remotely preserved on origin/master but can appear lost when using main. Integration must reconcile two diverged native UI implementations.

## Recommendation
When recovering native chat polish, compare 0c89afff against current main and port the intended behavior; do not assume switching to main includes that commit.
