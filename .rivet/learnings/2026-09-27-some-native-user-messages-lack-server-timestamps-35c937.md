---
title: Some native user messages lack server timestamps; do not stamp them at load
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/timing.rs
  - apps/native/src/model.rs
  - services/hub/cmd/mcp/conversation.go
promoted: false
---

# Some native user messages lack server timestamps; do not stamp them at load

## Observation
The live get_conversation textOnly response for this Codex session retained user_message items with no timestamp, while assistant/tool items carried times. textOnly preserves raw fields, so this is genuinely missing provider history. Native displays Time unavailable for unstamped messages rather than inventing their send time. On mid-turn attach, TurnClock can instead seed elapsed work from the first recorded assistant/tool event after the latest unstamped user prompt; the user message itself remains unstamped. This avoids restarting an already-running work timer at client launch.
