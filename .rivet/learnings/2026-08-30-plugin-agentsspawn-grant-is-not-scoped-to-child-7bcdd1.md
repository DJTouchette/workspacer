---
title: Plugin agents.spawn grant is not scoped to child facade authority
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/bus/rpc.go
  - services/hub/internal/bus/bus.go
  - services/hub/internal/plugin/manifest.go
  - services/hub/cmd/brain/facade.go
  - apps/desktop/src/main/services/claudeSpawn.ts
promoted: false
---

# Plugin agents.spawn grant is not scoped to child facade authority

## Observation
Plugin bus tokens can be granted the verb-only agents.spawn capability, and callerToolScopeCeiling deliberately returns no tier ceiling for plugin callers. Spawn providers then honor mcpFacade/toolScope/pluginTools by minting a child session facade token, defaulting legacy mcpFacade to operator.

## Impact
A plugin consented to call agents.spawn can ask for a child agent with operator-tier first-party tools and arbitrary pluginTools even though the plugin's own bus token does not have those capabilities. Directory ceilings may clamp some requests, but the shipped default ceiling allows operator and plugins have no caller-tier clamp.

## Recommendation
Treat child facade tier and pluginTools as separately consented plugin authority, or clamp plugin-initiated spawns to no facade/view tier unless the plugin manifest explicitly grants child tool delegation.
