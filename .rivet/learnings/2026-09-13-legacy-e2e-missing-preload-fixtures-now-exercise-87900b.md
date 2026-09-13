---
title: Legacy E2E missing-preload fixtures now exercise desktop-service bus fallbacks
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/src/backend/desktopServices.ts
  - apps/desktop/tests/e2e/fleetWorkflows.test.ts
  - apps/desktop/tests/e2e/taskInspector.test.ts
  - services/hub/internal/uploads/store.go
promoted: false
---

# Legacy E2E missing-preload fixtures now exercise desktop-service bus fallbacks

## Observation
createBridgedBackend overlays HOST_ONLY methods only when the preload supplies a function. fleetWorkflowRequest/dispatchHistoryRead also exist in the base desktopServices bus adapter, so omitting them from a fixture preload now retains a bus fallback rather than producing an absent method. Old workflow/task-inspector tests must model an old host's bus refusal explicitly; merely changing expected strings can hide fixture transport gaps. Mobile uploads separately moved from workspacer-uploads to workspacer-uploads-<uid> (uploads.DirName).
