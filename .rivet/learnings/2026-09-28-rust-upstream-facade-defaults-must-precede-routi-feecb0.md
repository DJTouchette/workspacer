---
title: Rust upstream facade defaults must precede routing
date: 2026-09-28
promoted: false
---

# Rust upstream facade defaults must precede routing

## Observation
The Go MCP spawnWithGrants resolves omitted claude.defaultModel/contextWindow and skipPermissions through config.get before forwarding to the bus. The Rust facade must do the same before the central routing clamp when the execution provider is remote; relying on node-local SpawnCoordinator defaults can disagree with the control-plane policy. Non-Claude model IDs must never inherit a Claude config default.
