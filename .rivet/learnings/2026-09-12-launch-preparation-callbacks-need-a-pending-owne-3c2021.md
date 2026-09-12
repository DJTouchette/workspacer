---
title: Launch preparation callbacks need a pending owner spawn, not the provider's ambient token
date: 2026-09-12
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - services/hub/internal/bus/launchintegration.go
  - services/hub/cmd/hub/launchintegration.go
  - services/hub/cmd/brain/launchintegration.go
  - apps/desktop/src/main/services/launchIntegrationCore.ts
promoted: false
---

# Launch preparation callbacks need a pending owner spawn, not the provider's ambient token

## Observation
Headless launch preparation cannot reuse the native hub owner token under the isolated Fly UID split. plugins.prepareLaunch is now a provider callback bound by router pending-call id, provider connection, active authenticated owner, and exact selected launchIntegrationId. Describe/prepare expose only that selected plugin's declaration and whitelisted LaunchContext; the hub self-client invokes its declared sidecar method. Revoked owners, another provider, swapped plugins, and completed calls fail. Native and headless reuse launchIntegrationCore validation and Codex route probing.
