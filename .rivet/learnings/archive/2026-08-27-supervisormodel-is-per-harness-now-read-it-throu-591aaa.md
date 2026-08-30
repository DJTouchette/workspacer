---
title: supervisor.model is per-harness now — read it through lib/supervisorModel, never inline
date: 2026-08-27
confidence: high
suggested_doc: supervisor-loop
related_paths:
  - apps/desktop/src/main/lib/supervisorModel.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - apps/desktop/src/main/services/claudeSpawn.ts
  - apps/desktop/src/main/services/mcpConfig.ts
  - apps/desktop/src/renderer/src/components/settings/SupervisorSection.tsx
promoted: true
promoted_to: fleet-manager
---

# supervisor.model is per-harness now — read it through lib/supervisorModel, never inline

## Observation
`config.supervisor.model` is a single field but the supervisor can run on claude/codex/opencode, and a model id is never portable between them. Two latent bugs came out of the inline `supCfg?.model` read in claudeSpawn.ts: (1) managedSpawn never read it at all, so `supervisor.model` was silently Claude-PTY-only — picking a codex supervisor model changed nothing; (2) a Claude supervisor launched from AskPane while `supervisor.provider` was codex would have inherited the codex id and 400'd. Resolution now lives in `apps/desktop/src/main/lib/supervisorModel.ts`: `supervisor.models[provider]` (the per-harness memory the settings picker writes) wins, then `supervisor.model` but ONLY when `supervisor.provider` matches, else undefined (= the CLI's own default, the one value valid everywhere).

## Impact
Any new supervisor spawn path that reads supervisor.model directly reintroduces the cross-harness 400. Also: the settings model dropdown is now keyed on the selected harness via the shared `renderer/src/lib/modelOptions.loadModelOptions` + `capsFor(provider).modelSource` — it used to call `claudeListModels()` unconditionally, which is what made codex show Claude models.

## Recommendation
Read supervisor models via resolveSupervisorModel(provider). Keep managedFacadeInstructions (managed/stream) and facadeSpawnArgs (PTY) in step — both now carry summarizerModel + pollSeconds + whether /supervise was installed for that harness.
