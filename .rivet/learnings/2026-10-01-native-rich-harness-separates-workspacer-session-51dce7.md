---
title: Native rich harness separates Workspacer sessions from provider subagents
date: 2026-10-01
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/harness.rs
promoted: false
---

# Native rich harness separates Workspacer sessions from provider subagents

## Observation
Inline child rendering needs two distinct fixture paths: a successful namespaced spawn_agent receipt naming a child session with matching parentSessionId, and provider-native subagent snapshot entries joined by toolUseId. Rich harness supplies running/completed native children, telemetry, and parent-scoped sessions.subagentConversation replay; ordinary child sessions keep sessions.conversation. Nonrich benchmarks retain their existing snapshots.

## Impact
A lone unfinished spawn tool cannot validate attached child rows or clickable replay.

## Recommendation
Capture native child rows with rich fixture and at least two sessions; exercise provider-native replay without manufacturing a Workspacer session ID.
