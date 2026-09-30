---
title: Spawn support differs from parsing legacy facade fields
date: 2026-09-30
promoted: false
---

# Spawn support differs from parsing legacy facade fields

## Observation
Both Rust and desktop accept legacy fleetFullAccess/mcpFacade/toolScope/pluginTools spellings without letting those caller fields select token authority. Rust session_facade and TS spawners mint operator with all plugins from host code; fleet bypass comes from live config plus manager lineage. toolScope can still travel as paired-dispatch metadata, and pluginTools is refused in paired launch despite being ignored locally. A live source read or pass-through literal is not proof that a parameter is functionally supported; disposition guards must distinguish these cases.
