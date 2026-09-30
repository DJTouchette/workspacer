---
title: Typed session fields must validate before precedence or engine requests
date: 2026-09-30
suggested_doc: claudemon-http-api
related_paths:
  - services/hub-rs/src/services/sessions.rs
  - services/hub-rs/src/services/sessions/params.rs
  - services/hub-rs/tests/engine_adapter.rs
promoted: false
---

# Typed session fields must validate before precedence or engine requests

## Observation
A real embedded-engine plus inert wrapper WS regression proved malformed gate on values became false commands, malformed answer alternatives were ignored by precedence and emitted PTY bytes, wrong cwd/sinceSeq fields silently broadened reads, invalid approval reason reached the daemon, and invalid sender attribution was accepted. An explicit per-method session validator now rejects all known wrong types before querying or mutating; null scalar defaults stay unchanged, null entries in the two Go []string answer DTOs normalize to empty text, and precedence applies only after validation. The same-wrapper delivery marker proves no malformed input remains queued. The owning adapter test passed after reproducing all failures; workflows7/config20 also passed the shared snapshot.

## Impact
A valid higher-priority carrier cannot justify ignoring malformed lower-priority fields; silent defaults can change a gate or execute input.

## Recommendation
Keep validation method-specific, preserve opaque fields, and use actual registered-handler negative controls plus same-stream delivery markers.
