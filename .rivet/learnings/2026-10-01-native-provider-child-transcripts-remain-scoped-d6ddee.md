---
title: Native provider child transcripts remain scoped to their parent
date: 2026-10-01
promoted: false
---

# Native provider child transcripts remain scoped to their parent

## Observation
Provider-native child IDs must read sessions.subagentConversation with parent sessionId and agentId; they are not selectable fleet session IDs. Native secondary request state uses subagent-history, normalizes canonical seq/items via bounded Transcript, aborts and removes it on parent selection, and relies on existing epoch/request-number fencing for disconnects and superseded child reads. Null read is an explicit unavailable state, not an empty successful transcript.
