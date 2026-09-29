---
title: Remote directory readiness must use process liveness and canonical deduplication
date: 2026-09-29
confidence: high
suggested_doc: hub-federation
related_paths:
  - services/hub-rs/src/services/remote_dispatch/readiness.rs
promoted: false
---

# Remote directory readiness must use process liveness and canonical deduplication

## Observation
Rust remote dispatch readiness skipped only mode=stopped and remote rows, so live shells (mode unknown), archived rows and desktop-ended states could enter its active project menu. It now uses the shared process-liveness predicate. Canonicalize configured and active directories before deduplicating so a live canonical spelling cannot replace configured-project provenance from a different symlink spelling. Canonical absolute names remain literal rather than trimming or lexically cleaning before symlink resolution.
