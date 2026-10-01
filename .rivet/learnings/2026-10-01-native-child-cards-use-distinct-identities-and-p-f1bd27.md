---
title: Native child cards use distinct identities and parent-scoped replay
date: 2026-10-01
promoted: false
---

# Native child cards use distinct identities and parent-scoped replay

## Observation
Native inline child cards project exact toolUseId anchors and successful Workspacer session receipts without treating provider thread IDs as session IDs. Bot identifies Workspacer children; SquareTerminal identifies provider-native children. Native replay uses sessions.subagentConversation under its selected owning parent; known membership, request epoch/sequence and selection fences prevent stale cross-parent data. Metadata-only updates invalidate virtual row heights and preserve scroll/drafts.
