---
title: Native timing must retain server stamps and observe turn completion separately
date: 2026-09-27
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/model.rs
  - apps/native/src/timing.rs
  - apps/native/src/ui.rs
  - apps/native/src/ui/tools.rs
  - services/claudemon/src/providers/mod.rs
promoted: false
---

# Native timing must retain server stamps and observe turn completion separately

## Observation
Daemon ConversationItem supplies optional RFC3339 timestamps for user/assistant/tool events, and managed adapters stamp their arrival. Native Item previously discarded them. Native Row now retains a parsed start timestamp, streaming fragments keep that first timestamp, and joined tool results retain their completion timestamp. An assistant message's timestamp is not a turn-finish timestamp: the native TurnClock observes working/pending→idle boundaries, keeps one start across queued messages/approvals, freezes durations, and refuses completion timing after a disconnect. Observed completions persist as bounded per-hub/per-session JSON caches under native-turn-timings with SHA-256 path components. Unknown historical durations are not reconstructed from assistant fragment timestamps.
