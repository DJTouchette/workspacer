---
title: Web parity guard mixes desktop routing with browser support and misses headless providers
date: 2026-09-11
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/scripts/web-capability-*
  - apps/desktop/src/renderer/src/backend/*
  - services/hub/cmd/brain/headless_completeness_test.go
  - services/hub/internal/bus/bus.go
promoted: false
---

# Web parity guard mixes desktop routing with browser support and misses headless providers

## Observation
The web API inventory contains absent optional desktop functions as well as HOST_ONLY functions with working web equivalents (plugin catalog, settings, library events). A green backendParity test only proves its routing buckets. Live authenticated /health exposes methodNames and confirmed 100 registrations but no git.stage/unstage/commit/push/commitDiff/commitNumstat, fs.watch/unwatch or claude.handoffAgentBrief. hello.methods is an AUTHORIZATION pattern set, not a registry. Plugin install HTTP routes already exist and require host owner, while inspect/update/remove/enable use the operator guard.
