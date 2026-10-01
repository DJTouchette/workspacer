---
title: Native ordinary-agent skill delivery crosses the provider first-turn boundary
date: 2026-10-01
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/tests/local_spawn.rs
  - apps/native/src/backend.rs
promoted: false
---

# Native ordinary-agent skill delivery crosses the provider first-turn boundary

## Observation
Native Backend::spawn routes through agents.spawn; SessionFacade installs the same hash-pinned collaboration assets as Electron and injects pointers plus authenticated identity. Codex role/skill instructions are prepended to the first turn, while Claude receives append-system-prompt. Verification must exercise parent-authenticated MCP spawn_agent with trackTask:false and no parentSessionId, not only owner bus agents.spawn.

## Impact
A bus-only child dispatch test misses parent identity derivation and provider instruction delivery.

## Recommendation
Keep local_spawn fake-provider coverage for instruction pointers, actual MCP child dispatch, lineage, queued first message, and child completion wakes.
