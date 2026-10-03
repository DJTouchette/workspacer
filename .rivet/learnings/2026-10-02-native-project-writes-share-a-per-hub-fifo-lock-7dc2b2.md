---
title: Native project writes share a per-hub FIFO lock in Backend
date: 2026-10-02
confidence: high
suggested_doc: config
related_paths:
  - apps/native/src/backend.rs
  - apps/native/src/features.rs
  - apps/native/src/projects.rs
promoted: false
---

# Native project writes share a per-hub FIFO lock in Backend

## Observation
Backend::project_write() returns an OwnedMutexGuard from a process-wide registry keyed by normalized bus URL (Weak entries); embedded backends get their own lock (one per owned hub). save_project holds it across config.get→patch→config.save→verify, so SaveProject/TouchProject keep separate request keys/receipts but never interleave. verify(Remove) now also requires absence from directories.favourites/recent, since a legacy-only project has no projects entry even when the save was skipped.

## Impact
Without the shared lock both reads see the same map and the later wholesale projects save silently drops the earlier pin/removal while both receipts succeed. Cross-process writers (desktop, other native instances) remain outside this lock: the pre-existing config read/save race.

## Recommendation
Any new native path that writes config.projects must go through save_project (or take backend.project_write()) for the whole round. tests/protocol.rs Hub::shared() serves several controllers on one fake hub for such tests.
