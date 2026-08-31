---
title: Capability clamp can be undone by provider default model resolution
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/bus/rpc.go
  - apps/desktop/src/main/services/claudeSpawn.ts
  - apps/desktop/src/main/lib/spawnModel.ts
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Capability clamp can be undone by provider default model resolution

## Observation
When agents.spawn capability is refused, sanitizeSpawnParams deletes the exact model and effort fields. Later, provider-side spawn code can resolve an omitted model to a configured/default model, for example desktop Claude resolves empty model to config.claude.defaultModel (opus[1m]) after the bus sanitizer has run.

## Impact
A directory capability ceiling can turn an explicit over-ceiling model into an omitted model, then the provider can restore a strong default outside the sanitizer's view. This also affects raw bus callers that omit model/capability under a capped cwd.

## Recommendation
Resolve effective provider/model/effort before the bus ceiling check, or have the sanitizer replace refused model/effort with an explicit safe model for the clamped capability rather than deleting and letting providers default later.
