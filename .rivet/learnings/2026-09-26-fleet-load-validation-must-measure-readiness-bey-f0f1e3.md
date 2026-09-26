---
title: Fleet load validation must measure readiness beyond spawn admission
date: 2026-09-26
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/shared/spawnTiming.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - apps/desktop/src/main/services/remoteTokens.ts
  - services/claudemon/src/daemon/spawn.rs
  - services/claudemon/src/daemon/mod.rs
promoted: false
---

# Fleet load validation must measure readiness beyond spawn admission

## Observation
The existing agent-load benchmark does not launch providers. Managed launch_total ends after daemon admission and client setup; daemon start_native_managed queues the first message and schedules the provider driver before returning, so HTTP success is not provider readiness or first output. Session token minting synchronously reads and rewrites the whole tokens.json through atomicWriteFileSync, including fsync. Separately, hook persistence awaits one blocking write per event and only warns on broadcast lag, so missed events are not recovered there.

## Impact
Current synthetic improvements cannot establish startup latency or loss-free sustained fleet operation. Synchronous token I/O is a candidate burst bottleneck, and persistence lag is a correctness risk to exercise; neither has been measured as the next dominant bottleneck.

## Recommendation
Add repeatable isolated spawn-through-first-output and streaming load scenarios, including slow consumers, reconnects, stop/resume, event-loop delay and persistence-loss counters. Profile token writes before changing their concurrency/durability contract.
