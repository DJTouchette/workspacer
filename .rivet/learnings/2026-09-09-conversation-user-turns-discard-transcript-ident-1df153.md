---
title: Conversation user turns discard transcript identity
date: 2026-09-09
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - services/claudemon/src/session/conversation.rs
  - apps/desktop/src/main/services/sessionStore/conversationApplier.ts
  - apps/desktop/src/renderer/src/types/claudeSession.ts
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
promoted: false
---

# Conversation user turns discard transcript identity

## Observation
The claudemon transcript format has a row uuid, but ConversationItem::UserMessage serializes only text and timestamp. The desktop ConversationTurn likewise has no message/request id and deduplicates recent user messages by content/timestamp, so a task provenance link cannot currently name an exact durable user turn.

## Impact
Automatic task capture needs an explicit request identifier at the delivery/transcript boundary; a title/text hash would duplicate on retries and collide on repeated messages.

## Recommendation
Mint and persist a request id at successful manager-message admission, and carry it through the conversation item plus task provenance; make resolver writes idempotent on that id.
