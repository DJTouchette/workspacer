---
title: title_island_springs_into_notices_and_lets_go flakes under parallel test load
date: 2026-10-06
confidence: medium
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/island.rs
promoted: false
---

# title_island_springs_into_notices_and_lets_go flakes under parallel test load

## Observation
On unmodified HEAD 6a5aceb7 the native UI test failed 1 of 3 default (parallel) runs at
`src/ui.rs:6428` (the reduce-motion snapped island bounds) and passes alone and with
`--test-threads=1` (as CI runs it). Treat a lone failure there under parallel load as
the known flake, not a regression; rerun it alone before chasing it.
