---
title: Fleet card actions must not select-scroll on mousedown; card sends borrow the chat owner
date: 2026-09-06
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/components/FleetDeck.tsx
  - apps/desktop/src/renderer/src/components/AgentCard.tsx
  - apps/desktop/src/renderer/src/hooks/useSessionChatController.ts
promoted: false
---

# Fleet card actions must not select-scroll on mousedown; card sends borrow the chat owner

## Observation
Chromium reproduced a card Send mousedown selecting a worker and triggering virtualizer scroll before mouseup, so the click landed on the scroll container and no message request ran. Enter reached the provider boundary successfully but the old lastAssistant preview excluded user turns. Fleet menu and composer actions must isolate mousedown from selection. Cards now borrow the owning ClaudePane send handler and its conversation/pending projection keyed by session plus pane, retaining the existing FIFO echo reconciliation; no card transcript store or second viewer is created.
