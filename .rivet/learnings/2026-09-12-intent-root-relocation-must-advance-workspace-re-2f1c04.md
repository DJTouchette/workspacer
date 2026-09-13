---
title: Intent root relocation must advance workspace revisions, not execution identities
date: 2026-09-12
confidence: high
suggested_doc: config
related_paths:
  - apps/desktop/src/main/services/intentProjectStore.ts
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/shared/intentProject.ts
promoted: false
---

# Intent root relocation must advance workspace revisions, not execution identities

## Observation
Intent work creation and launch packets use workspace.projectRoot, while linked execution sessions retain independent cwd and immutable revision references. Stable intent project identity can live in dedicated project/repository tables. Relocating one repository must update current root and insert a new intent revision for every associated workspace in one transaction, so stale directions and workspace drafts are fenced; existing session cwd, historical snapshots, and context packets must remain unchanged. Config.yaml projects remain a separate directory-keyed fleet-settings map.
