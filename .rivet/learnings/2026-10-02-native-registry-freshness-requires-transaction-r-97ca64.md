---
title: Native registry freshness requires transaction revisions and alias-wide identity updates
date: 2026-10-02
confidence: high
suggested_doc: config
related_paths:
  - apps/native/src/backend.rs
  - apps/native/src/features.rs
  - apps/native/src/projects.rs
  - apps/native/src/ui/projects.rs
promoted: false
---

# Native registry freshness requires transaction revisions and alias-wide identity updates

## Observation
Request-start numbers across project reads and writes do not order snapshot freshness. Native project reads must participate in the per-hub read/patch/save/verify barrier, and snapshots need completion revisions allocated under that barrier. Imported equivalent project-map keys can coexist with conflicting metadata; removal must protect/check every same_dir alias, while pin and recency may update only identity fields on every alias without merging metadata.
