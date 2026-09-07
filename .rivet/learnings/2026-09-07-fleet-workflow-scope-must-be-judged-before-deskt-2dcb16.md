---
title: Fleet workflow scope must be judged before desktop narrows it
date: 2026-09-07
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowService.ts
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Fleet workflow scope must be judged before desktop narrows it

## Observation

The MCP facade is a shared host connection. Explicit workflow spawns must request their maximum toolScope before the hub ceiling runs; desktop workflow logic may only narrow the received scope. Reassigning operator in desktop after the bus clamp would undo the ceiling. Pinned wake validation must read history snapshots rather than relying on restored session.resultSchema.

## Impact

A workflow definition must not widen existing routing authority, and restart must not silently lose its result contract.

## Recommendation

Keep the real authenticated dispatch-chain fixture with routing.select and a view ceiling; retain the test that removes session resultSchema before a real wake.
