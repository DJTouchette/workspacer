---
title: Desktop agents.sendMessage silently drops fromSessionId that the brain honours
date: 2026-08-29
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/enrich.go
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Desktop agents.sendMessage silently drops fromSessionId that the brain honours

## Observation
services/hub/cmd/brain/handlers.go:1058 prepends fleetSenderHeader() — "[fleet] session:<id> (<label>) says:\n" (cmd/brain/enrich.go:67) — when agents.sendMessage carries fromSessionId. The desktop twin, apps/desktop/src/main/services/hubCapabilities.ts:485, destructures only { sessionId, text } and drops the field. The MCP facade's sendMessageIn (cmd/mcp/main.go:1368) advertises fromSessionId to every agent as "the message is delivered with a header naming you as the sender".

## Impact
A dispatched worker messaging its manager arrives ANONYMOUS in the normal desktop case, and attributed only on a headless node. The tool description promises attribution that the primary provider does not implement. grep -rn fleetSenderHeader apps/desktop returns nothing.

## Recommendation
Mirror handlers.go:1058 in hubCapabilities.ts's agents.sendMessage; keep the header string as a shared twin (it borrows [fleet] and session:<id> from fleetMessages.ts but is deliberately NOT a FleetMessageKind, so parseFleetMessage must keep not round-tripping it).
