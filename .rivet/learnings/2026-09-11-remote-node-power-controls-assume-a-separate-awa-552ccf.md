---
title: Remote node power controls assume a separate awake hub and symmetric Fly capabilities
date: 2026-09-11
confidence: high
suggested_doc: remote-mobile
related_paths:
  - services/hub/internal/nodes/*.go
  - services/hub/cmd/hub/nodes.go
  - services/hub/cmd/hub/mobile.html
  - apps/desktop/src/renderer/src/components/RemoteNodesBar.tsx
promoted: false
---

# Remote node power controls assume a separate awake hub and symmetric Fly capabilities

## Observation
The existing desktop RemoteNodesBar and mobile /m already expose nodes.wake and nodes.sleep, but Supervisor stores flyapi.Client per node and Node.Wakeable/NodeView.Wakeable imply both start and stop. The liveness path probes a single provider-attach brain.info, not a federation peer or arbitrary laptop. Extending this to laptop wake requires independent wake/sleep capabilities, explicit target readiness attribution, and a controller/relay reachable while the target sleeps; simply adding a Wake button to a client connected to the sleeping target cannot work.

## Recommendation
Preserve the existing Fly controls while introducing provider-neutral per-action capabilities. Treat wake delivery and verified readiness separately, and never apply Fly failed-wake automatic shutdown to a personal laptop.
