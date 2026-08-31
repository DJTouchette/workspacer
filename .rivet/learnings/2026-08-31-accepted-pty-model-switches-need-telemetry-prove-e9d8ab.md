---
title: Accepted PTY model switches need telemetry provenance, not field clearing
date: 2026-08-31
author: codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/session/state.rs
  - apps/desktop/src/main/services/claudeSessionStore.ts
promoted: false
---

# Accepted PTY model switches need telemetry provenance, not field clearing

## Observation
The Phase 5 owner and desktop mirror clear model/window fields only at acceptance, but the ordinary status-line ingest/apply paths blindly replace them on every later frame. A delayed pre-switch Claude status frame can therefore restore the old 200k window after an accepted opus[1m] switch, and that stale value then propagates through snapshots and persistence/restart projections.

## Impact
A session accepted at 1M can regress to 200K in owner and UI telemetry after the next status tick. Any live control mirrored beside asynchronous provider telemetry needs an epoch/provenance fence that survives subsequent frames until truthful confirmation.

## Recommendation
Stamp accepted live selections with an epoch/provenance barrier; suppress only incompatible model/window telemetry while preserving unrelated telemetry, and release the barrier only on a compatible provider confirmation. Persist the canonical selection independently.
