---
title: Bridged backend drops optional preload-only APIs unless host-classified
date: 2026-09-06
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/tests/fleetReview.test.tsx
promoted: false
---

# Bridged backend drops optional preload-only APIs unless host-classified

## Observation
createBridgedBackend begins with the web backend and only overlays LOCAL_TERMINAL and HOST_ONLY methods. Optional Fleet review methods added solely to preload were undefined in the default desktop bus mode until both methods were placed in HOST_ONLY.

## Impact
A renderer feature test that directly stubs window.electronAPI can pass while default desktop bus mode silently feature-detects the API as absent.

## Recommendation
For host-only renderer features, add their preload methods to HOST_ONLY and test the production component against createBridgedBackend with a local preload stub and an old-preload absence case.
