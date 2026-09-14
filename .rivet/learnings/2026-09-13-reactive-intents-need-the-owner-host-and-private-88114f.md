---
title: Reactive intents need the owner host and private headless delivery bridge
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/headless/desktopHost.ts
  - services/hub/cmd/brain/intentsteering.go
promoted: false
---

# Reactive intents need the owner host and private headless delivery bridge

## Observation
IntentWorkspaceStore is shared between native Electron and the brain's headless TS child. Native session events capture observations directly; headless observations arrive via internal.captureIntentSessions. Headless agent effects must use hostCall on the private pipe (intent.send/intent.interrupt), not native claudemonSessionClient. Activation implemented only in the renderer would stop with the view and diverge between native and remote clients.
