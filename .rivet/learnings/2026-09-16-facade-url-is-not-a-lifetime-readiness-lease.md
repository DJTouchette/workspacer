---
title: Facade URL is not a lifetime readiness lease
confidence: high
impact: A healthy startup can be followed by a disconnected or replaced facade; minting from the cached URL leaks a bearer into a spawn that cannot use it.
related_paths:
  - services/hub/cmd/brain/facade.go
  - apps/desktop/src/main/services/mcpFacadeDaemon.ts
suggested_doc: mcp-tool-facade
---

The configured MCP facade URL proves only where to probe. Every spawn must
re-check the health document's exact service, bind, hub identity,
`hubConnected`, and `pluginCatalogReady` before minting or injecting a session
token. Recovery must stay uncached too: a later healthy probe should restore
facade wiring without restarting the brain.
