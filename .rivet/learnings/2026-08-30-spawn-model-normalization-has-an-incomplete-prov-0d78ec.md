---
title: Spawn model normalization has an incomplete provider gate
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/lib/spawnModel.ts
  - apps/desktop/src/main/lib/spawnModel.test.ts
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Spawn model normalization has an incomplete provider gate

## Observation
The shared desktop resolveSpawnModel helpers normalize legacy [1m]/-1m suffixes regardless of provider, while the Go facade already gates its Claude default selection with providerIsClaude. This can mutate valid Codex, OpenCode, or Pi model IDs that happen to end in -1m.

## Impact
Provider-specific model IDs must remain opaque outside the selected provider's vocabulary.

## Recommendation
Keep the legacy Claude normalization inside a provider === 'claude' gate and pin non-Claude -1m IDs byte-for-byte in tests.
