---
title: Session update context publication must stay on the blocking resolver path
date: 2026-08-31
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/session/usage.rs
promoted: false
---

# Session update context publication must stay on the blocking resolver path

## Observation
GET session snapshots already resolve usage from the transcript on spawn_blocking, while session.update SSE previously serialized SessionState directly. Publishing resolved_context_window consistently therefore requires one shared state projection and running the event projection on the blocking pool too; otherwise snapshots and updates can disagree or block an async runtime worker.

## Impact
Clients consuming REST and SSE must receive identical owner-authored requested_selection and resolved_context_window facts without introducing a second resolver or synchronous transcript I/O on the runtime.

## Recommendation
Keep published_session_value shared by REST snapshots and serialize_session_update, and derive resolved_context_window only from usage_for_session.context_limit.
