---
title: Session retention and manager recovery docs had contradictory lifetime claims
date: 2026-09-26
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/daemon/mod.rs
  - services/claudemon/src/store/schema.rs
  - apps/desktop/src/main/services/managerReplacementState.ts
promoted: false
---

# Session retention and manager recovery docs had contradictory lifetime claims

## Observation
Daemon maintenance evicts stale stopped rows and prunes old SQLite session/event rows beyond a newest-100 floor; archive is not an indefinite retention promise. ManagerReplacementState persists eligible parent and isWakeTarget metadata in manager-replacements.json, while live tombstone maps remain process-local. Recovery overlays attribution and must defer liveness to current daemon evidence. Schema migration is transactional through v8 with unversioned rollback-compatible additions, not one-shot V1.

## Recommendation
Keep archive display, retention, durable lineage, and live liveness separate; verify old append-only notes against executable paths before repeating them.
