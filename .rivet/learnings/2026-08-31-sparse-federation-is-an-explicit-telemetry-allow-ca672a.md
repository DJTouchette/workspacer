---
title: Sparse federation is an explicit telemetry allowlist
date: 2026-08-31
author: Codex
confidence: high
suggested_doc: hub-federation
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/claudeSessionStore.test.ts
  - services/hub/cmd/brain/enrich.go
promoted: false
---

# Sparse federation is an explicit telemetry allowlist

## Observation
`upsertSparseRemoteSession` is an explicit allowlist mapper, not a spread. The brain's sparse compatibility row now includes camelCase `statusLine`, but the desktop mapper omitted it while separately preserving `requestedSelection`; rich federation tests therefore could not cover the sparse-only loss.

## Impact
Headless peer model telemetry can disappear at the desktop federation seam even though the brain projected it correctly, making a sparse peer row look windowless/model-less.

## Recommendation
When adding a field to the brain's sparse compatibility overlay, add it to the desktop sparse mapper and pin a sparse federated regression with deliberately incompatible owner selection/status telemetry so any accidental local fence is also visible.
