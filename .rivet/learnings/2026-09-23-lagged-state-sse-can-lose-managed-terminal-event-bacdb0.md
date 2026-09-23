---
title: Lagged state SSE can lose managed terminal events without resync
date: 2026-09-23
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/daemon/api.rs
  - apps/desktop/src/main/services/claudemonEventBridge.ts
promoted: false
---

# Lagged state SSE can lose managed terminal events without resync

## Observation
SessionStore state broadcast capacity is 256 (services/claudemon/src/session/store.rs:22). daemon/api.rs event_stream logs BroadcastStream lag and returns None without notifying clients or refreshing state. Desktop claudemonEventBridge.ts handles SessionEnd as the only managed-process termination signal and has neither sequence-gap tracking nor a state refresh callback. Losing the final event during overload can leave stale live state; unlike conversation delta gaps this path has no equivalent recovery. This is a source-confirmed conditional failure, not reproduced live.

## Impact
Under a slow subscriber or event burst, terminated managed sessions can remain visually live and retained, compounding perceived slowdown.

## Recommendation
Add explicit resync signaling and authoritative refresh on lag/reconnect. Validate with a bounded broadcast overflow test containing the final SessionEnd. Include this with snapshot transport optimization.
