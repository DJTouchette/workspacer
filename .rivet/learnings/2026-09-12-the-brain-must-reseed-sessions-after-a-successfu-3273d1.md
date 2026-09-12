---
title: The brain must reseed sessions after a successful SSE connection
date: 2026-09-12
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - services/hub/cmd/brain/events.go
  - services/hub/cmd/brain/claudemon.go
  - services/hub/cmd/brain/events_reseed_test.go
promoted: false
---

# The brain must reseed sessions after a successful SSE connection

## Observation
The full scratch image starts its hub before claudemon. runSessionStore formerly seeded once before connecting; if that first HTTP request failed and a session's only edge happened before the stream retry, the brain never learned that session's filesystem root. The hub's separate bridge could still make it visible in the browser. Subscribe-first plus seed after each successful SSE connection now restores missed sessions; queued events are then re-read through getSession. The earlier cwd cache fix removes delay after a row is present, but this startup/reconnect hole was the additional source of smoke-test denials.
