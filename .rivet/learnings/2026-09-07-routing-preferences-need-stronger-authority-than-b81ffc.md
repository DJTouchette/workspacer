---
title: Routing preferences need stronger authority than IsTrusted and retained host model classifications
date: 2026-09-07
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/preferences.go
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Routing preferences need stronger authority than IsTrusted and retained host model classifications

## Observation
bus.CallerIdentity.IsTrusted also admits scoped operator tokens and untokened loopback. Routing preferences now use an explicit AuthenticatedHost bit excluding peers and scoped tokens. MCP separately checks the authenticated static facade credential: otherwise its outbound host bus connection launders operator-tier authority. Managed row replacements retain the immutable host model classification in Matrix so deleting a strong row cannot lower canonical ceiling enforcement.

## Recommendation
Keep the hub handshake, MCP HTTP/SSE host-gate tests, and preference-to-select_model consumer fixtures together when changing this boundary.
