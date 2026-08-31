---
title: Prompt-first Claude spawns must not participate in model-default persistence
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/lib/modelOptions.ts
  - apps/desktop/src/main/lib/spawnModel.ts
promoted: false
---

# Prompt-first Claude spawns must not participate in model-default persistence

## Observation
App.tsx routes both dialog submissions and command-palette prompt-first spawns through handleSpawnAgent. Prompt-first intentionally omits model so main can resolve the saved canonical model/context pair, but the shared callback previously translated that omission into defaultModel empty plus contextWindow null before spawning.

## Impact
A command-palette dispatch could erase the saved Opus 1M pair and make the triggering spawn race against the destructive save.

## Recommendation
Build the remembered model patch only when a spawn carries an explicit model selection; keep omitted model keys absent, and verify the downstream spawn resolver serializes the retained pair.
