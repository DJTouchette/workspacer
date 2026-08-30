---
title: A third ProjectIdentity copy (ipcTypes.ts AppConfig.projects) was also missing yolo+delivery
date: 2026-08-23
confidence: high
suggested_doc: config
related_paths:
  - apps/desktop/src/main/shared/ipcTypes.ts
  - apps/desktop/src/main/services/configService.ts
  - apps/desktop/src/renderer/src/hooks/useConfig.ts
promoted: true
---

# A third ProjectIdentity copy (ipcTypes.ts AppConfig.projects) was also missing yolo+delivery

## Observation
Beyond the known drift between configService.ts and useConfig.ts (missing `delivery`), there was a THIRD hand-kept copy: main/shared/ipcTypes.ts's AppConfig.projects was an inline object type (not even named) that was missing BOTH `yolo` and `delivery` — despite its own doc comment explicitly saying it exists to prevent exactly this kind of drift ("a field that lives only in configService's own Config is invisible to every consumer of AppConfig"). ipcTypes.ts is already the established main↔renderer shared-type file (renderer's tsconfig.json includes ../../main/shared, and many other types like RecentAgentSession/ClaudeProfile/FleetMessage are already imported this way from both sides) — configService.ts and useConfig.ts just weren't using that pattern for ProjectIdentity.

## Impact
Closes the drift class this file's own comment warned about — a future config-validation pass reading AppConfig would have silently stripped yolo/delivery on any project going through that path.

## Recommendation
Fixed 2026-08-23: ProjectIdentity is now declared ONCE in main/shared/ipcTypes.ts (exported, all fields incl. yolo/delivery/worktreeSetup/plugins), and both configService.ts and useConfig.ts import it (useConfig.ts re-exports it as `export type { ProjectIdentity }` so existing `from '../hooks/useConfig'` call sites are unaffected). AppConfig.projects now references the same type instead of a separately-hand-kept inline object. Any future per-project field should be added to ipcTypes.ts's ProjectIdentity only.

## Disposition
Promoted into .rivet/context/domains/config.md (ProjectIdentity is now single-sourced, 2026-08-23), consolidated with 764f0e.
