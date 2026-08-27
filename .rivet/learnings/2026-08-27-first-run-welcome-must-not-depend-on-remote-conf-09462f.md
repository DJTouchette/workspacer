---
title: First-run welcome must not depend on remote config writes
date: 2026-08-27
author: codex
confidence: high
suggested_doc: desktop-remote-client-mode
related_paths:
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/contexts/ConfigContext.tsx
  - apps/desktop/src/renderer/src/backend/remoteBackend.ts
promoted: false
---

# First-run welcome must not depend on remote config writes

## Observation
In desktop remote-client mode, renderer config reads and writes go through the remote hub. If that configured server is offline, ConfigContext falls back to DEFAULT_CONFIG and config.save returns the previous snapshot after warning, so onboardingDismissed cannot change. A first-run welcome gate that depends only on persisted config can therefore keep remounting and block access to Settings / Connect to Server.

## Impact
Remote-client users pointed at a down Fly/ORD node can be trapped behind the welcome modal exactly when they need local settings controls to recover.

## Recommendation
Keep a local, in-session dismissal path for blocking UI such as first-run welcome, or route recovery settings through host-local APIs instead of remote config calls.
