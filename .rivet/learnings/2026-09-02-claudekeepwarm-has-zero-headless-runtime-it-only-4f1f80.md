---
title: claude.keepWarm has zero headless runtime — it only exists inside Electron main
date: 2026-09-02
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/main/services/keepWarmService.ts
  - apps/desktop/src/main/services/keepWarmLogic.ts
  - services/hub/internal/limits/window.go
  - services/claudemon/src/daemon/heartbeat.rs
promoted: false
---

# claude.keepWarm has zero headless runtime — it only exists inside Electron main

## Observation
grep -rn "keep_warm|keepWarm|KeepWarm" across services/claudemon/src and services/hub returns zero runtime hits (only cross-referenced pure-math comments in services/hub/internal/limits/window.go noting the 5h-window logic is twinned with keepWarmLogic.ts). KeepWarmService (apps/desktop/src/main/services/keepWarmService.ts) runs entirely in Electron main: it ticks every 60s, reads configService.getConfig().claude?.keepWarm directly, and calls claudemon's stateless HTTP routes (GET /usage, GET /usage/report, POST /heartbeat) as a dumb executor with zero config awareness of its own. So claude.keepWarm.enabled has literally no effect in a headless `workspacer serve`/hub deployment — keepWarmService.start() is simply never invoked there.

## Impact
Anyone building a new claudemon-adjacent toggle who copies the keepWarm pattern as a template will accidentally build something Electron-only with no headless equivalent, even though services/hub/cmd/brain/config_defaults.json (the shared default source, go:embed'd by the brain) makes it LOOK like the config is already headless-aware.

## Recommendation
Before reusing keepWarm as a template for a new toggle, check whether the new feature's actual mechanism lives inside claudemon itself (headless-agnostic by construction, like account_usage::spawn_poller) or is externally orchestrated like keepWarm (Electron-only unless a Go-side ticker is built to match). The two need different toggle transports: an externally-orchestrated feature needs a decider on both the Electron and Go sides; a daemon-internal feature just needs one flag read once at claudemon boot (env var or argv, both spawn sites — apps/desktop/src/main/services/claudemonDaemon.ts and services/hub/cmd/workspacer/serve.go+child.go — already support adding either trivially).
