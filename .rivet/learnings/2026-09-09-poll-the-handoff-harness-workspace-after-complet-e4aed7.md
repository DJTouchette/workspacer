---
title: Poll the handoff harness workspace after completion status
date: 2026-09-09
confidence: high
suggested_doc: renderer-live-state-hooks
related_paths:
  - apps/desktop/tests/e2e/managerHandoff.test.ts
promoted: false
---

# Poll the handoff harness workspace after completion status

## Observation
CI run 34432913510 on fd79c30e passed native Windows artifacts but failed the renderer handoff browser test: the completion label was visible while a one-shot fleetHarness.agentRecords read still returned s-manager and successor count 0. The harness refreshes that accessor in a React effect keyed by manager.agents; useAgentManager schedules setAgents and publishes the bind response separately. The test must poll its complete workspace/session/pane/count/start assertion instead of treating label visibility as proof that the harness accessor has refreshed.

## Impact
This timing failure leaves the overall CI red even when all native Windows validation tests pass. The e2e assertion values are unchanged; no production behavior or validator code changed.

## Recommendation
Await the actual identity snapshot in browser fixtures. A persistent wrong identity must still time out and fail. Do not replace the assertion with a fixed delay or weaken the expected successor/count.
