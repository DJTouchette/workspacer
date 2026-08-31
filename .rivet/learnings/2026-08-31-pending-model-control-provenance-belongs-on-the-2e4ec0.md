---
title: Pending model-control provenance belongs on the private queue entry
date: 2026-08-31
author: Codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/daemon/api.rs
promoted: false
---

# Pending model-control provenance belongs on the private queue entry

## Observation
Claudemon's pending input queue is private in-memory state (`SessionStore.pending_messages`), while persisted/session snapshots expose only the ordinary public session shape. Structural PTY model controls and chat currently converge to the same `String` queue, so provenance must live on a private queue-entry type rather than be inferred from `/model ` text or added to the snapshot/schema.

## Impact
Queue-capacity handling can either silently discard an accepted structural model switch or accidentally make ordinary chat non-evictable.

## Recommendation
Classify pending entries at the structural model-control acceptance boundary, keep ordinary `/message` payloads evictable regardless of text, and make enqueue failure caller-visible before requested-model persistence.
