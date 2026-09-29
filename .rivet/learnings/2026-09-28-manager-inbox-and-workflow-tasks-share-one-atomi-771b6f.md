---
title: Manager inbox and workflow tasks share one atomic revision boundary
date: 2026-09-28
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/task_store.rs
  - services/hub-rs/src/services/manager_requests.rs
promoted: false
---

# Manager inbox and workflow tasks share one atomic revision boundary

## Observation
dispatchHistoryStore.ts persists both tasks and manager requests in dispatch-history.json version1. Request resolution preflights every update CAS and reference mapping before any task/content mutation; successful resolution deletes userContent only in that same transaction. beginDelivery durably sets delivery unknown before IO and pending/unknown/accepted attempts cannot replay. Even a conversational resolved request makes later ordinary tracked dispatch require a committed taskId. Task ownership must be rechecked after acquiring the same history lock, and dependency acceptance binds exact dispatch/session/result evidence.

## Impact
Separate Rust inbox and task files or naive per-intent writes would lose atomicity, leak partial tasks on conflict, permit stale-owner edits or replay uncertain user prompts.

## Recommendation
Use TaskStore.transaction for both documents, capture workflow pins before the history lock, preserve distinct request and task revisions, and never interpret unknown delivery as permission to retry.
