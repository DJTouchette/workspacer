---
title: Intent project identity is still directory-keyed despite ProjectIdentity naming
date: 2026-09-12
confidence: high
suggested_doc: config
related_paths:
  - apps/desktop/src/main/shared/intentWorkspace.ts
  - apps/desktop/src/main/shared/ipcTypes.ts
  - apps/desktop/src/renderer/src/lib/projectRegistry.ts
promoted: false
---

# Intent project identity is still directory-keyed despite ProjectIdentity naming

## Observation
ProjectIdentity in main/shared/ipcTypes.ts contains visual/configuration metadata, not a stable ID. renderer/lib/projectRegistry.ts unions config.projects keys, legacy favorite/recent roots, scripts, and widgets to discover projects; patchProject writes the entire projects map because configService replaces it wholesale. IntentWorkspace stores projectRoot directly and immutable revision/context packets capture it. Stable project/root relocation therefore needs an explicit new durable identity mapping and must preserve historical paths rather than rename config keys alone.
