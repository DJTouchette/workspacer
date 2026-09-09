---
title: Local desktop account checks require a desktop-owned daemon environment
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/claudemonDaemon.ts
  - apps/desktop/src/main/services/providerReadinessRuntime.ts
  - apps/desktop/src/main/services/managedSpawn.ts
promoted: false
---

# Local desktop account checks require a desktop-owned daemon environment

## Observation
Profile-less managed spawns inherit claudemon's environment, not the current desktop's; an adopted healthy local daemon can therefore use another account. Provider readiness restricts native desktop inference to a child started by this desktop, includes its private pid in result identity, and refuses it if the desktop environment changed since daemon spawn (private hash only). Adoption/runtime health semantics are unchanged; getClaudemonReadinessOwner returns null for adopted or absent owners.
