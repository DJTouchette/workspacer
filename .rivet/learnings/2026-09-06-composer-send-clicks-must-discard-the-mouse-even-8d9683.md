---
title: Composer send clicks must discard the mouse event before calling the shared send handler
date: 2026-09-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/components/claude/Composer.tsx
  - apps/desktop/src/renderer/src/panes/SessionChatView.tsx
promoted: false
---

# Composer send clicks must discard the mouse event before calling the shared send handler

## Observation
The retained chat handleSend accepts optional cardText. Passing Composer onSend directly as onClick sends React MouseEvent as cardText and throws at trim(), while Enter succeeds. Chromium Fleet checks reproduced this at both widths. Composer now invokes onSend() without arguments; unit tests assert zero arguments and browser tests cover click, pending echo, acknowledgement and failed draft retention.

SessionChatView must also consume the ChatSendResult returned by handleSend: the shared owner restores rejected drafts, but ignoring its return value hid the reason. The composer now displays the returned error using the existing session UI state store; it clears on retry and survives Back.
