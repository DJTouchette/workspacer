---
title: Native embedded daemon requires explicit external ownership in the headless launcher
date: 2026-09-27
suggested_doc: workspacer-serve-cli
related_paths:
  - services/hub/cmd/workspacer/serve.go
promoted: false
---

# Native embedded daemon requires explicit external ownership in the headless launcher

## Observation
workspacer serve --external-claudemon uses selected loopback API port and validates plain ok plus x-workspacer-maintenance: 1 before launching children. It omits daemon init, DB resolution, binary requirement, restart and shutdown. Hub, facade and brain remain launcher-owned; the JSON ready banner does not prove brain registration. WORKSPACER_PARENT_PID plus piped stdin now enables parentwatch shutdown for serve itself.

## Recommendation
Native hosts should wait for brain.info after the ready banner, keep the stdin pipe until shutdown, stop the owned stack before embedded daemon, and preserve stream-versus-PTY answer behavior when using direct channels.
