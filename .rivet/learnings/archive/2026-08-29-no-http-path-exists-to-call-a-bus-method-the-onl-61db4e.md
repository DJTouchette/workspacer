---
title: No HTTP path exists to call a bus method; the only unauthenticated manager-wake is claudemon POST /sessions/:id/message
date: 2026-08-29
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/internal/bus/bus.go
  - services/hub/cmd/workspacer/fleetcmd.go
  - services/hub/cmd/mcp/main.go
  - apps/desktop/src/main/services/mcpFacadeDaemon.ts
  - services/claudemon/src/daemon/api.rs
promoted: true
promoted_to: hub-bus-control-plane
---

# No HTTP path exists to call a bus method; the only unauthenticated manager-wake is claudemon POST /sessions/:id/message

## Observation
The hub bus mounts only /bus (WebSocket) and /health (internal/bus/bus.go:615); every other route is a plugin/static AddRoute in cmd/hub. There is no HTTP endpoint that performs a capability call, which is why workspacer fleet quiescence exists as a bespoke CLI seam for ONE read (its own header says "a bus method is not reachable from a shell"). Meanwhile claudemon's API on 127.0.0.1:7891 has NO auth — AllowedHosts is a DNS-rebind guard only — so POST /sessions/:id/message is a credential-less one-line wake for any local process. And mcpFacadeDaemon.ts:158 deliberately never sets WKS_MCP_TOKEN while cmd/mcp defaults -untokened to operator, so a credential-less MCP client on 127.0.0.1:7897 gets the full operator tool set.

## Impact
Anything asking "how do I poke a session from a script/webhook/CI" has exactly three answers today: unauthenticated curl at claudemon, an MCP client at the untokened facade, or a hand-rolled /bus WebSocket. The workspacer CLI has no `call` verb (subcommands: serve|plugin|status|token|fleet|jobs|install-cli) and `fleet` has only `quiescence`.

## Recommendation
Add new shell-reachable reads/writes as `workspacer fleet <verb>` subcommands in cmd/workspacer/fleetcmd.go — that file already owns token resolution, host/port flags and the exit-code contract. Do not assume a hub HTTP POST exists.
