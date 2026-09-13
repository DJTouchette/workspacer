---
title: Conversation polling skips new items after stream coalescing
date: 2026-09-12
suggested_doc: remote-mobile
related_paths:
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/session/conversation.rs
promoted: false
---

# Conversation polling skips new items after stream coalescing

## Observation
get_conversation filters ?since via items_skip(first_seq,len,since), assuming retained items have contiguous sequence numbers. ConversationStore::push coalesces assistant chunks and updates per-item item_seqs, so a prompt at seq1 plus hundreds of chunks at seq2..N leaves two items. Poll since2 skips the growing reply; later follow-up messages are also skipped. Filter stored item_seqs > since under the same log lock instead. Existing items_skip_window test incorrectly describes all 4 coalesced items as older than since400; counter-based tests must exercise actual store and HTTP route.
