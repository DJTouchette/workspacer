---
title: Native client must reconcile bus snapshots with in-flight events
date: 2026-09-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/native
promoted: false
---

# Native client must reconcile bus snapshots with in-flight events

## Observation
The native GPUI prototype tests exercise the actual hub WebSocket vocabulary. A sessions.snapshots reply can arrive after newer agent.snapshot events, and sessions.conversation can race raw deltas. Buffer projected fleet updates and bounded conversation deltas while reads are in flight, replay after the snapshot, and reject completions from older connection or selection generations. Coalesced conversation seq values count events, not retained rows.

## Impact
Without these rules reconnects and session switches can show stale content or append another session transcript.

## Recommendation
Keep the real-WebSocket race tests and GPUI draft/virtualization tests in apps/native in the client feedback loop.
