---
title: Performance recovery must distinguish buffered state from newer snapshots
date: 2026-09-23
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - apps/desktop/src/main/services/claudemonEventBridge.ts
  - services/claudemon/src/session/conversation.rs
  - services/claudemon/src/providers/mod.rs
  - services/hub/cmd/brain/events.go
promoted: false
---

# Performance recovery must distinguish buffered state from newer snapshots

## Observation
The state SSE endpoint now emits session.resync on broadcast lag. Desktop reconciliation subscribes first, fetches state_only rows including empty/archived sessions, and compares updated_at at nanosecond precision against frames arriving during the fetch. A received frame is not necessarily newer: buffered pre-lag frames can arrive while the authoritative fetch runs. Bounded per-connection checkpoints also reject older frames arriving after reconciliation. Headless brain handles the marker with authoritative snapshot publication. Transcript tail reads now have a 256KiB per-file pass budget and eight concurrent sessions, directory scans cache for two seconds, unchanged subagent files back off for five seconds. Model-cache misses share in-flight success/failure, and canceling one owner releases waiters; optional Codex bundled probing has a two-second kill-on-drop timeout.

## Impact
Lost final exits now have a recovery path without stale buffered updates undoing it. Catalog fanout and transcript catch-up no longer launch duplicate provider probes or read an entire backlog per pass. Idle subagent spend may arrive up to five seconds later; ordinary main transcript ticks remain 400ms.

## Recommendation
Keep recovery requests state-only where possible, retain the generation guard and full timestamp precision, and preserve partial JSON rows when tuning tail budgets. Validate concurrent catalog cancellation, timeout cleanup, broadcast overflow, and stale-buffer ordering with the focused tests.
