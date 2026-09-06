---
title: Dispatch history identity must be stamped before the shared facade bus connection
date: 2026-09-06
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/mcp/main.go
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Dispatch history identity must be stamped before the shared facade bus connection

## Observation
The MCP facade multiplexes session credentials over one host-token connection. Recent agents uses callerSessionID(ctx) to stamp dispatchOwnerSessionId and a json-excluded respawn source field; the bus must strip both fields for scoped operator, plugin and federation callers as well as untrusted callers. A claimed parent or role alone does not establish dispatch ownership. The local history read is IPC-only and must be listed in bridgedBackend HOST_ONLY; web and remote return explicit unavailable without a new bus capability.

## Impact
Prevents forged task/retry grouping and silent empty history in desktop bus mode.

## Recommendation
Keep the caller-stamp, source-owner validation and actual bridged factory tests together when changing spawn/history surfaces.
