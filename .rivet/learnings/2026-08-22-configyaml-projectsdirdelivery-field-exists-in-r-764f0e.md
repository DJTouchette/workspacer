---
title: config.yaml projects.<dir>.delivery field exists in renderer type but not main-process type
date: 2026-08-22
confidence: high
suggested_doc: config
related_paths:
  - apps/desktop/src/main/services/configService.ts
  - apps/desktop/src/renderer/src/hooks/useConfig.ts
  - apps/desktop/src/renderer/src/lib/fleetManager.ts
promoted: false
---

# config.yaml projects.<dir>.delivery field exists in renderer type but not main-process type

## Observation
The per-project ProjectIdentity interface is duplicated three times (apps/desktop/src/renderer/src/hooks/useConfig.ts, apps/desktop/src/main/services/configService.ts, apps/tui/src/projects.rs) plus doctrine text in fleetManager.ts referencing projects[<dir>].delivery. The renderer's useConfig.ts:198 has `delivery?: 'pr' | 'local'` (Fleet Manager delivery-mode doctrine field) but the main-process configService.ts copy (lines 51-97) does NOT declare it — it stops at yolo/worktreeSetup/plugins. Since config.yaml is untyped YAML at runtime this doesn't break anything (the field round-trips fine), but it's a silent type-definition drift between the two ProjectIdentity copies.

## Impact
A future TS refactor that starts validating config.projects against configService.ts's ProjectIdentity type would silently strip `delivery` on write/normalize, breaking the Fleet Manager's per-project delivery-mode doctrine (rule 6 in fleetManager.ts) without any visible error.
