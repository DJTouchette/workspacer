---
title: Routing preferences need separate consent labels
date: 2026-09-07
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - apps/desktop/src/renderer/src/lib/pluginPermissions.ts
  - apps/desktop/src/renderer/tests/pluginPermissions.test.ts
  - services/hub/cmd/hub/main.go
promoted: false
---

# Routing preferences need separate consent labels

## Observation
Hub-native routing preference methods bypass the desktop and brain capability registries, so the renderer drift guard is the only consent-label check that exposed their raw IDs.

## Impact
Plugins otherwise show internal method IDs for routing preference reads, validation, changes, reset, and previews.

## Recommendation
Keep CAP_LABELS synchronized with RegisterLocal and RegisterLocalIdent methods and test read/preview severity separately from mutation labels.
