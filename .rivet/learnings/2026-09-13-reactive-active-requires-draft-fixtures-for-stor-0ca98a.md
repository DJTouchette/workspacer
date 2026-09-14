---
title: Reactive Active requires Draft fixtures for storage-only intent tests
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/ipc.test.ts
  - apps/desktop/src/main/services/claudeSessionStore.test.ts
  - apps/desktop/src/main/services/intentWindowsFiles.test.ts
  - apps/desktop/scripts/test-desktop-host.mjs
promoted: false
---

# Reactive Active requires Draft fixtures for storage-only intent tests

## Observation
The schema-6 reactive intent create path activates Active work and requires an outcome plus success criteria. Legacy storage, IPC, and capture tests creating empty Active work fail before reaching their target behavior, including Windows-only tests and scripts/test-desktop-host.mjs. Storage-only fixtures should use Draft; real automation tests must supply complete intent and stub effects.

## Recommendation
Run the full main suite and Windows intent job after lifecycle changes, not just files named intent.
