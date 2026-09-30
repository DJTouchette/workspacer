---
title: Spawn spelling guards need live source closure beyond historical vocabulary
date: 2026-09-30
related_paths:
  - contracts/spawn-parameter-keys.json
  - services/hub-rs/src/auth.rs
  - tools/capability-source-check/src/policy.rs
  - apps/desktop/src/main/services/spawnKeyDrift.test.ts
promoted: false
---

# Spawn spelling guards need live source closure beyond historical vocabulary

## Observation
Historical spawnKeys46 no longer covered5 roots inspected or stripped by current Rust spawn/workflow code. A real registered-provider bus test against the old cached hub accepted uppercase INTENTS unchanged. The new language-neutral51-key registry reserves all traced roots without granting authority; historical Go/vocabulary bytes remain unchanged. Rust source closure and desktop AST roots now reject new unmapped fields, deleted registry keys, blind scans and unexplained reservations.

## Recommendation
Keep normalization separate from canonical spawn path selection: terminal raw14 corpus preserves literal tilde and Unicode while spawn plans require canonical absolute paths. Do not infer a parameter is accepted or authoritative merely because its spelling is reserved.
