---
title: Browser watch leases must pass through both native and headless providers
date: 2026-09-12
confidence: high
related_paths:
  - apps/desktop/src/main/services/fileWatchService.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/filewatch.go
promoted: false
---

# Browser watch leases must pass through both native and headless providers

## Observation
webBackend renews a stable watchId on reconnect and every minute. Both fs.watch providers must forward this identity so renewal is idempotent; ignoring it leaks a legacy refcount per renewal. Native file watches observe the parent directory with a filename filter to survive atomic saves, while directory watches must observe their own children unfiltered.
