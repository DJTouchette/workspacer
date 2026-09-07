---
title: Retained chat setters must notify remounted consumers
date: 2026-09-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/hooks/useSessionChatUiState.ts
promoted: false
---

# Retained chat setters must notify remounted consumers

## Observation
The retained UI map previously updated its original hook instance only. Fleet virtualization can unmount a sending card during expansion; a late request result then left the remounted card stuck Sending and could overwrite a newer draft from the old closure. Per-entry subscriptions now notify current consumers, and functional updates read the shared field value. This keeps the existing bounded store and termination fences.
