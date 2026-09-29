---
title: A generic HTTP operation route needs a closed operation authority table
date: 2026-09-29
author: codex
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - contracts/http-route-registry.json
  - apps/desktop/src/main/services/httpRouteRegistry.test.ts
  - apps/desktop/tests/support/rustHttpSource.ts
promoted: false
---

# A generic HTTP operation route needs a closed operation authority table

## Observation
The Rust plugin HTTP router consolidates several concrete Go paths into POST /plugins/:operation. Route-name completeness alone would silently accept a newly added mutable match arm. The portable HTTP registry now checks the exact operation set, unknown-operation refusal, operator guard before effects and the stronger host gate for reload. It also binds all70 current routes to actual method handlers and applied guard chains, checks host-only event twins, and rejects duplicate-path extra verbs or unparsed method-router suffixes. Test-module exclusions are derived from cfg(test) declarations and cannot hide an additional production import.

## Impact
Porting a source guard by copying its old path list would miss changes introduced by the new router shape even while runtime HTTP smoke stayed green.

## Recommendation
Update the reviewed registry and pair-bound proof together for intentional route or authority changes. Keep mutation tests for removed guards, newly added arms/verbs, stale twins and merely-defined-but-unapplied confinement layers.
