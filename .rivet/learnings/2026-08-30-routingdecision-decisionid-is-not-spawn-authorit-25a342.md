---
title: routing.decision decisionId is not spawn authority proof
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/hub/routingselect.go
  - services/hub/internal/routing/decisionlog.go
  - cmd/mcp/main.go
promoted: false
---

# routing.decision decisionId is not spawn authority proof

## Observation
routing.select creates a random decisionId and logs/publishes it, but agents.spawn treats decisionId as an ordinary caller-supplied metadata string. sanitizeSpawnParams does not include decisionId in SpawnCeilingRequest and does not verify it against a hub-owned decision record before enforcing ceilings.

## Impact
A requested-vs-routing-decided exemption based on decisionId would be forgeable or replayable unless backed by trusted state, freshness, cwd/caller/model/capability binding, and consumption semantics. As currently wired it cannot safely distinguish caller request from routing's own decision.

## Recommendation
Do not add a decisionId-based ceiling bypass. Make routing.select ceiling-aware and return an already-capped actionable decision, or add a hub-owned verifier with strict binding if provenance is needed for audit only.
