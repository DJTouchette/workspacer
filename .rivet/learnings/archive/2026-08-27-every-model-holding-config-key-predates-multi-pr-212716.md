---
title: Every model-holding config key predates multi-provider and holds a CLAUDE id
date: 2026-08-27
confidence: high
suggested_doc: agent-spawn
promoted: true
promoted_to: agent-spawn
---

# Every model-holding config key predates multi-provider and holds a CLAUDE id

## Observation
claude.defaultModel ('opus[1m]'), supervisor.model, supervisor.summarizerModel ('sonnet') and agents.autoTitle.model ('haiku') all ship Claude ids because they predate multi-provider support. Only claude.defaultModel was ever gated on provider (lib/spawnModel, and the Go facade's providerIsClaude). The other three were read unconditionally on paths that can spawn codex. Fixed 2026-08-27 by giving each a per-harness sibling map (supervisor.models from 508d8272, plus supervisor.summarizerModels, agents.autoTitle.models, agents.managerModels) resolved through main/lib/roleModels, with main/shared/modelVocabulary as the cross-harness oracle.</observation>
<parameter name="impact">A foreign model id on a spawn is a 400 at best and a silently-wrong model at worst. Any NEW config key holding a model must be born per-harness — a single field is only ever right for one harness.</impact>
<parameter name="recommendation">Resolve new role models through lib/roleModels perHarnessModel; never read a *.model config field inline in a spawn path. resolveSpawnModel is the last-line guard and drops a demonstrably-foreign id to undefined (= the CLI's own default).</recommendation>
<parameter name="related_paths">["apps/desktop/src/main/lib/roleModels.ts", "apps/desktop/src/main/shared/modelVocabulary.ts", "apps/desktop/src/main/lib/spawnModel.ts"]
