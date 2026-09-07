---
title: Execution engine admission belongs in claudemon
date: 2026-09-07
promoted: false
---

# Execution engine admission belongs in claudemon

## Observation
Desktop managedSpawn and headless brain converge on daemon spawn-managed. Native provider argument construction stays in daemon/spawn.rs; the engine registry must wrap that call and existing SessionStore controls. Codex already atomically publishes under its native generation lock; a registry event fence must not hold that lock while starting a driver. Engine compatibility identity is a persisted lineage, not a build hash.
