---
title: Configured Fly projects were not directory-picker roots until an agent was running there
date: 2026-09-11
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/cmd/brain/fsguard.go
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/providers.go
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# Configured Fly projects were not directory-picker roots until an agent was running there

## Observation
Live mobile spawn verification found config.projects contains /data/repos, /data/repos/preheat and /data/repos/workspacer, while fs.listDir(/data/repos) was denied and /data/home had no visible subdirectories. Both brain and desktop browseRoots admitted only home plus live-agent/content-store roots. Added narrowly scoped spawnSetupRoots for fs.listDir and providers.listModels to include explicitly configured absolute project roots before the first agent runs. fs.read/fs.write and library/workflow roots are unchanged, with a test proving an inactive configured project is browsable but not readable/writable via content APIs.

## Recommendation
Use spawnSetupRoots only for project selection and model discovery. Do not turn remembered/configured projects into general content-access grants. Upgrade the brain binary alongside the hub when changing headless folder/model handlers.
