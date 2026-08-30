---
title: mcp facade daemon never sets WKS_MCP_TOKEN — desktop-launched instance is always unauthenticated loopback
date: 2026-07-11
confidence: high
related_paths:
  - apps/desktop/src/main/services/mcpFacadeDaemon.ts
  - services/hub/cmd/mcp/main.go
  - services/hub/cmd/brain/parity_test.go
promoted: true
---

# mcp facade daemon never sets WKS_MCP_TOKEN — desktop-launched instance is always unauthenticated loopback

## Observation
apps/desktop/src/main/services/mcpFacadeDaemon.ts spawns services/hub/cmd/mcp's `mcp` binary with only --addr 127.0.0.1:7897 and --hub <bus url> (plus --token for the outbound hub-bus auth). It never passes -mcp-token nor sets WKS_MCP_TOKEN. checkBindPolicy() in main.go only requires a token for non-loopback binds, so this is legal and intentional — the facade's own inbound HTTP surface (/mcp, /sse) is always open/unauthenticated when launched by the desktop app, relying purely on 127.0.0.1 binding as the boundary. Also: spawnAgentIn.mcpFacade (spawn_agent's mcpFacade:true param) only works when the facade is the desktop-hosted one — services/hub/cmd/brain/parity_test.go's spawnParamsDeclined map explicitly documents that brain (headless serve mode) declines to mirror mcpFacade/mcpItemIds because headless has no facade URL / no buildSessionMcpConfig writer to wire.

## Impact
If anyone changes mcpFacadeDaemon.ts to bind on a non-loopback address (e.g. for remote/mobile access) without also generating and passing a WKS_MCP_TOKEN, the facade will refuse to start (checkBindPolicy fails closed) — so it's safe, but the fix must add token plumbing to mcpFacadeDaemon.ts, not just change the addr. Separately, don't expect spawn_agent's mcpFacade:true to work from a headless `workspacer serve`/brain deployment — it's a desktop-only capability by design.

## Recommendation
When adding remote/mobile reachability for the mcp facade, generate a random WKS_MCP_TOKEN in mcpFacadeDaemon.ts (mirroring how hubDaemon.ts handles HUB_TOKEN) rather than assuming loopback-only forever.

## Disposition
Superseded 2026-08-15 by the facade tool-tier work: auth is now tiered per request (authGate; per-session scoped tokens; unknown token = 401). The surviving kernel — untokened loopback = operator back-compat, non-loopback bind requires token plumbing in mcpFacadeDaemon.ts, brain declines mcpFacade — is folded with corrections into .rivet/context/modules/mcp-tool-facade.md.
