---
title: First message must not wait for terminal viewer attachment
date: 2026-09-08
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/renderer/tests/components/ClaudePane.test.tsx
promoted: false
---

# First message must not wait for terminal viewer attachment

## Observation
Hosted CI 34301729259 failed ordinary New Agent Codex first-message delivery (151/152 browser tests passed). A bounded repeated Chromium probe reproduced it, and a deterministic ClaudePane test with null useClaudeSpawn sessionId reproduced zero claudeMessage calls. The composer is visible while xterm font loading and attachClaude are pending. handleSend previously took its raw PTY fallback, whose viewer key did not exist yet. For an already-created session, attachSessionId is the canonical message target until the attached sessionId arrives; the latter must win once available.

## Impact
Fast first messages could be displayed optimistically without reaching the daemon; extending the test timeout does not repair delivery.

## Recommendation
Use sessionId ?? attachSessionId for message delivery while preserving daemon rejection and failure handling. Keep the null/present attachment regression and ordinary New Agent browser send assertions.
