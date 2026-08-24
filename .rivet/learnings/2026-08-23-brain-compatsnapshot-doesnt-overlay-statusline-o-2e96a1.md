---
title: Brain compatSnapshot doesn't overlay statusLine or totalToolCalls to camelCase
date: 2026-08-23
confidence: high
suggested_doc: workspacer-serve-cli
related_paths:
  - services/hub/cmd/brain/enrich.go
  - services/hub/cmd/hub/mobile.html
  - services/hub/cmd/brain/parity_test.go
promoted: false
---

# Brain compatSnapshot doesn't overlay statusLine or totalToolCalls to camelCase

## Observation
services/hub/cmd/brain/enrich.go's compatSnapshot() maps claudemon's raw snake_case session row onto the desktop's camelCase ClaudeSessionSnapshot field names (sessionId, ambientState, usage, pendingApproval/Questions, lastActivity) — but it never adds a camelCase `statusLine` or `totalToolCalls` key. For a session with no desktop attached (pure brain/headless), mobile.html's `s.statusLine` is undefined and `s.totalToolCalls` is undefined; the raw claudemon fields ride along unrenamed as `s.status_line` (with its own snake_case internals like `received_at`, `total_output_tokens`) and `s.tool_calls`. Anything reading the camelCase names (e.g. the new stall-detection port in mobile.html's progressFingerprint/statusLineAlive) silently gets 0/undefined for headless-only sessions unless it also falls back to the snake_case originals.

## Impact
Any mobile.html feature keyed on `s.statusLine.*` or `s.totalToolCalls` degrades silently for brain-only (no-desktop) sessions — exactly the "phone monitors the fleet with no desktop open" scenario the mobile client exists for. The stall detector's progressFingerprint/statusLineAlive now read `s.status_line`/`s.tool_calls` as an explicit fallback (see attentionFor's progressFingerprint + statusLineReceivedAt in mobile.html) specifically to avoid this trap; other future features reading statusLine or totalToolCalls off a session row need the same fallback or a fix to compatSnapshot's overlay list.

## Recommendation
Either add `statusLine` (from status_line) and `totalToolCalls` (from tool_calls) to compatSnapshot's overlay in enrich.go, extending snapshotFieldsRequired/parity_test.go accordingly, or keep documenting the snake_case fallback pattern for any new mobile.html code that reads these fields.
