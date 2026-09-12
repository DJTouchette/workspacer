---
title: Isolated MCP outbound scope strips local workflow provenance
date: 2026-09-12
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/mcp/main.go
  - deploy/fly/combined
promoted: false
---

# Isolated MCP outbound scope strips local workflow provenance

## Observation
The combined supervisor intentionally uses a separate scoped MCP outbound hub credential. sanitizeSpawnParams strips dispatchOwnerSessionId and retrySourceSessionId from every scoped connection and fleetWorkflows.request refuses them, even after the facade authenticates a per-session HTTP token. Browser owner service wiring alone therefore cannot make manager workflow tools function on the isolated server. Infrastructure session-identity delegation must be explicit and remain independent of authenticatedHost and existing profile/full-access ceilings.
