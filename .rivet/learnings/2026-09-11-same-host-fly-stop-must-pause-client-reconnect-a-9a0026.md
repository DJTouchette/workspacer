---
title: Same-host Fly stop must pause client reconnect and strengthen advisory quiescence job checks
date: 2026-09-11
confidence: high
suggested_doc: fly-node-deploy
related_paths:
  - services/hub/cmd/hub/machinepower.go
  - services/hub/cmd/hub/quiescence.go
  - services/hub/cmd/hub/mobile.html
  - apps/desktop/src/renderer/src/backend/hubBusClient.ts
  - deploy/fly/combined/*
promoted: false
---

# Same-host Fly stop must pause client reconnect and strengthen advisory quiescence job checks

## Observation
Fly HTTP autostart supports combined hub/worker, but every ordinary /bus reconnect is also a wake request. The mobile PWA polls sessions.snapshots every 25 seconds and hub client activity counts all calls, so open polling clients prevent fleet quiescence. Existing quiescence ignores shell jobs and only protects jobs within a lookahead; using it to stop a host with no external scheduled wake must additionally protect all scheduled/running jobs. Implemented opt-in self machine.power/machine.stop and close code 4001 client pause separately from remote node registry.

## Recommendation
Do not enable Fly traffic-based autostop for agent workloads. Keep scheduled jobs awake until an external wake scheduler exists. For future user-idle detection distinguish interactive input from passive app polling before changing quiescence semantics.
