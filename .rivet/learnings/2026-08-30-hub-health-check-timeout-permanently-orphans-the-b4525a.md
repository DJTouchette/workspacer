---
title: Hub health-check timeout permanently orphans the desktop from a hub that comes up late
date: 2026-08-30
confidence: high
suggested_doc: hub-process-supervision
related_paths:
  - apps/desktop/src/main/services/hubDaemon.ts
  - apps/desktop/src/main/index.ts
  - apps/desktop/src/main/lib/daemonUtils.ts
promoted: false
---

# Hub health-check timeout permanently orphans the desktop from a hub that comes up late

## Observation
startHub()'s waitForHealth gives up after HEALTH_TIMEOUT_MS=5000 (hubDaemon.ts:38). If the hub binds late (observed: 15s under a boot storm — 3x Electron GPU-process SEGVs in Mesa libgallium + systemd-coredump churn + claude:spawn taking 11s), the rejection is permanent: readyPromise stays rejected (the child never exited, so the exit-driven restart path never clears it), and index.ts's .then chain — startHubClient, startFederationBridge, startMcpFacade, facade-token sweep, startFullAccessGrantSync — is skipped entirely. Result: the hub process is healthy on :7895 but the desktop reports "hub not connected" on every layout:set/nodes:list for the whole session. Nothing retries; only an app restart recovers.

## Impact
One slow boot (GPU driver crash-loop, disk storm, cold cache) silently costs the whole session: plugins, remote sharing, federation, MCP facade, layout persistence, and full-access grant reconciliation — while the hub itself is fine. The warn notification is accurate but never self-heals.

## Recommendation
On health-check timeout, don't give up while the child is still alive: keep polling in the background, and on late success run the post-ready chain and clear the 'hub-start' notification (notifySystem key exists for exactly this). Alternatively/additionally raise HEALTH_TIMEOUT_MS. Separate issue worth knowing: dev Electron 43 on this Arch box segfaults its GPU process 3x per launch (Mesa gallium 26.1.1) then falls back to SwiftShader — every launch is software-rendered and boot-stormy.
