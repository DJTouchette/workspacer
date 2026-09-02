---
title: A new window-rendering surface must go in bridgedBackend HOST_ONLY, not just the web stub
date: 2026-09-02
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
  - apps/desktop/src/main/shared/usageReport.ts
promoted: false
---

# A new window-rendering surface must go in bridgedBackend HOST_ONLY, not just the web stub

## Observation
claudemon's GET /usage/report is loopback-confined (internal/capspec/httproutes.go RouteLoopbackConfined) and the hub exposes NO bus capability for it, so only Electron main can fetch it. The trap: createBridgedBackend starts from the WEB backend and overlays only LOCAL_TERMINAL + HOST_ONLY. The default desktop launch is bridged (bus) mode, so a preload method that has a web stub but is NOT in HOST_ONLY silently gets the stub on the desktop. backendParity.test.ts's "no preload method vanishes" check does NOT catch this: it only flags methods missing from BOTH the web backend and the overlays. A stub that answers `null` passes that test while deleting the feature everywhere except pure-IPC mode. Same shape as the remote-access control plane that went missing before. Also note keepWarmHeartbeats is in exactly this state today (web stub returns [], not in HOST_ONLY), so the Settings heartbeat list is empty in bus mode.

## Impact
Any future "read the daemon directly" surface (mobile /m, wks-tui, an inspector cold-start fill) hits this. The failure is invisible: no error, no console warning, no bus event, and every component test stays green because the component is tested against the IPC shape.

## Recommendation
When adding a preload method that reads claudemon over loopback: add it to HOST_ONLY in bridgedBackend.ts, give webBackend an honest null/empty stub with a HUB-TODO, and write a test asserting HOST_ONLY contains the name (see backendParity.test.ts "the usage report comes from the daemon on desktop"). A generic parity test cannot infer which stubs are lies.
