---
title: routing.select is now ceiling-aware, which is what let the shipped default ceiling go back to `frontier`
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/routing/policy.go
  - services/hub/internal/routing/ceiling.go
  - services/hub/internal/routing/routing.default.yaml
  - services/hub/cmd/hub/routingselect.go
promoted: false
---

# routing.select is now ceiling-aware, which is what let the shipped default ceiling go back to `frontier`

## Observation
The shipped `ceilings.default` was `frontier_plus` (caps nothing) purely because `roles.judge` is frontier_plus: a `frontier` default made routing.select advise Fable and the spawn gate strip it — advise-then-refuse, once per judge. Select now runs the SAME Matrix.CheckSpawn the gate runs, against the same canonical cwd (Request.CanonicalCwd, `json:"-"`, filled by the handler with bus.CanonicalizeRoot), at step 7b — AFTER the mode shift, so a spend_down promotion cannot climb past a cap, and BEFORE step 8 resolves a model. The gate still clamps independently. Default is now `frontier`. CONSEQUENCE TO KNOW: under it a judge resolves to codex gpt-5.6-sol at high effort (mixed profile's `frontier`), and spend_down's frontier_max promotions for implementer/complex_fixer/diagnostician land back on frontier. The old coherence test ("CheckSpawn never refuses ResolveCapability's answer") is what forced the permissive default; it was restated one level up as "CheckSpawn never refuses what Select ADVISES", which is strictly stronger.

## Impact
Explicitly rejected: a decisionId-based sanitizer bypass. decisionId is caller-supplied, published on the open-by-decision routing.decision event, forgeable and replayable — a ceiling a caller can talk past by asserting the system told it to reads as protective while being worse than none. It stays AUDIT CORRELATION ONLY. Do not revisit without hub-held trusted decision state.
