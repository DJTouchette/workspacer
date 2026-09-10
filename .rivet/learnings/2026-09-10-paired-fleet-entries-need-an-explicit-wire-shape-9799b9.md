---
title: Paired fleet entries need an explicit wire shape and local action routing
date: 2026-09-10
suggested_doc: hub-federation
related_paths:
  - services/hub/cmd/brain/fleetmsg.go
  - apps/desktop/src/main/services/pairedDispatch.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
promoted: false
---

# Paired fleet entries need an explicit wire shape and local action routing

## Observation
The recovered fleetEntry Go struct had no JSON tags. Hosted local-MCP to paired-brain fixture reached remote isolated spawn but the desktop rejected every returned update because Label and SessionID did not match label and sessionId. Explicit camelCase tags fixed the actual wire route. Paired local card IDs also need local-origin qualification in the default web/bridged backend, because pairing is not a named Go federation peer.

## Impact
A successful remote spawn, healthy connection, and passing independent Go/TS unit suites do not prove a local manager wake. The shared fixture must exercise serialized brain output and the default renderer action route.

## Recommendation
Keep the real paired dispatch fixture and wire-field test, plus the paired backend routing test. Preserve local task/request ownership and derive remote recipients only from durable origin records.
