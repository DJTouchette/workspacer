---
title: Fleet chat reuses the mounted pane through a stable portal destination
date: 2026-09-06
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/components/claude/RetainedSessionChat.tsx
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
promoted: false
---

# Fleet chat reuses the mounted pane through a stable portal destination

## Observation
App keeps all AgentWorkspaceView/ScrollContainer trees mounted. Fleet can display the existing SessionChatView by moving a stable portal container, leaving ClaudePane as sole spawn/terminal/subscription owner. Changing the React portal container would instead remount descendant HTML/review state; state-preserving DOM moveBefore avoids iframe document reloads. Fleet capture keys must yield for interactive chat descendants and action buttons.
