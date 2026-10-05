---
title: New spawn key or republish site trips four reviewed registries
date: 2026-10-05
confidence: high
suggested_doc: registration-checklists
related_paths:
  - contracts/spawn-parameter-*.json
  - apps/desktop/src/main/services/busPublicationSites.test.ts
promoted: false
---

# New spawn key or republish site trips four reviewed registries

## Observation
Adding agents.spawn key autoTitle required: contracts/spawn-parameter-keys.json (keys + reservations + alias case), contracts/spawn-parameter-support.json (case + rustOnly + semantics when desktop ignores it), hard-coded counts in tools/capability-source-check/src/policy.rs (51->52), tests/spawn_support.rs and apps/desktop spawnKeyDrift/spawnSupport tests, and hub-rs tests/spawn_key_boundary.rs require_every aliases. Any new self.publish(row) call site in hub-rs needs a PASSTHROUGH entry in apps/desktop/src/main/services/busPublicationSites.test.ts (key 'services/<file>: <arg>', count, proof substrings of the compacted source).

## Recommendation
Run capability-source-check --check and the desktop main vitest suite after touching spawn params or hub publish sites.
