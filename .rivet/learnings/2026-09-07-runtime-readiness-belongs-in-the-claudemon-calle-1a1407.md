---
title: Runtime readiness belongs in the claudemon caller inventory
date: 2026-09-07
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/agentRuntimeStatus.ts
  - services/hub/internal/capspec/claudemoncallers_test.go
promoted: false
---

# Runtime readiness belongs in the claudemon caller inventory

## Observation

The agentRuntimeStatus.ts module added in master1d265ea5 builds the claudemon /health URL using PORTS.claudemonApi and probes it through probeHealth. It is an actual claudemon HTTP caller, not merely a port declaration. Integrating it requires a callerScan entry with a floor of one in claudemoncallers_test.go.

## Impact

Without this entry, the cross-language caller coverage guard fails and future health-route drift would be untracked.

## Recommendation

Keep readiness in claudemonCallers and scan its PORTS.claudemonApi template URL; do not exempt it as a non-caller.
