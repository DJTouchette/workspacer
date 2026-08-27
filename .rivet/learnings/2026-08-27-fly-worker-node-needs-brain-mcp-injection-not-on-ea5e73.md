---
title: Fly worker node needs brain MCP injection, not only the mcp binary
date: 2026-08-27
author: codex
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - deploy/fly/node/Dockerfile
  - deploy/fly/node/entrypoint.sh
  - services/hub/cmd/mcp/main.go
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/parity_test.go
  - services/claudemon/src/daemon/spawn.rs
  - services/claudemon/src/providers/codex.rs
promoted: false
---

# Fly worker node needs brain MCP injection, not only the mcp binary

## Observation
The Fly worker node runs claudemon plus brain in provider-attach topology and does not start a local hub. The MCP facade binary can technically forward to HUB_BUS_URL if it is packaged and started on 127.0.0.1:7897, but headless-spawned fleet workers still will not see workspacer MCP tools unless services/hub/cmd/brain mirrors the desktop facade injection path and passes claudemon's managed spawn facade fields (mcp URL and first-turn instructions) or Claude --mcp-config argv. Today parity_test.go deliberately declines mcpFacade/toolScope/pluginTools because desktop owns per-session token minting and config writing.

## Impact
Adding /usr/local/bin/mcp or starting port 7897 on the Fly node would make manual local MCP clients possible, but Fleet Manager workers spawned through the headless brain would still come up without mcp__workspacer__ tools.

## Recommendation
Treat headless fleet MCP as a feature: build/copy cmd/mcp into the node image, supervise it against HUB_BUS_URL/HUB_TOKEN, then add a brain-owned facade injection path with an explicit auth decision for node-local sessions (probably local untokened loopback or a node-static token first, scoped per-session tokens later).

## Status
Implemented on 2026-08-27: the Fly node image now builds/copies `mcp`, the entrypoint supervises it, and the brain mints per-session scoped facade tokens for `mcpFacade`/`toolScope`/`supervisor` spawns. The remaining operational wrinkle is token topology: a provider-scoped `HUB_TOKEN` is still right for `brain`, while the facade may need `WKS_MCP_HUB_TOKEN` for its outbound bus calls.
