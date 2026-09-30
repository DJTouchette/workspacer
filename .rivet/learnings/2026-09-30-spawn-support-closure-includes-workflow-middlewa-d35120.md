---
title: Spawn support closure includes workflow middleware separately from legacy grant stamps
date: 2026-09-30
confidence: high
related_paths:
  - contracts/spawn-parameter-support.json
  - tools/capability-source-check/src/spawn_support.rs
  - apps/desktop/tests/support/spawnSupport.ts
promoted: false
---

# Spawn support closure includes workflow middleware separately from legacy grant stamps

## Observation
The ordinary Rust agents.spawn trace and desktop inner-handler AST omit expectedTaskRevision even though both workflow admission implementations read it. The new symmetric support guard derives actual workflow caller roots and literal iterator keys, rejects dynamic keys, and proves insertion of a new middleware field fails closure. Source observation remains distinct from supported/refused/host-derived/ignored semantics: remoteCwd is supported by the paired adapter despite absent ordinary Rust reads; local facade tier/plugin and obsolete grant stamps do not grant caller authority.

## Recommendation
Maintain explicit reviewed dispositions and exact inverse differences; new implementation reads must not automatically count as support. Preserve legacy Go bytes and canonical key ownership.
