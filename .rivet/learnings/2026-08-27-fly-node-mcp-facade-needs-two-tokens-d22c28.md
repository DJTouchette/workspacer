---
title: Fly node MCP facade needs two tokens
date: 2026-08-27
promoted: false
---

# Fly node MCP facade needs two tokens

## Observation
A Fly provider-attach node now runs a loopback mcp facade and the brain injects per-session facade tokens for mcpFacade/toolScope/supervisor spawns. The brain still connects with HUB_TOKEN as a provider, but the facade's outbound bus connection may need WKS_MCP_HUB_TOKEN when HUB_TOKEN is provider-scoped; otherwise agents can authenticate to the local facade and still have hub calls denied.
