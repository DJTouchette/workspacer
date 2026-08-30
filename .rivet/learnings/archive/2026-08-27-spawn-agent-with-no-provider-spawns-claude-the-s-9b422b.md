---
title: spawn_agent with no provider spawns Claude — the supervisor's digest workers relied on it
date: 2026-08-27
confidence: high
suggested_doc: supervisor-loop
promoted: true
promoted_to: fleet-manager
---

# spawn_agent with no provider spawns Claude — the supervisor's digest workers relied on it

## Observation
The /supervise skill and both facade prompt builders (mcpConfig facadeSpawnArgs + managedFacadeInstructions) told the supervisor to spawn its transcript-digest worker with a MODEL but no PROVIDER. spawn_agent defaults to Claude, so a codex supervisor dispatched Claude summarizers — which is why supervisor.summarizerModel's claude-only 'sonnet' default looked correct: it was right by accident, and only because the setting it named was never actually reaching a codex spawn. Fixed by adding summarizerProvider to both builders (one shared mcpConfig.summarizerSpawnNote) so the digest worker follows its supervisor's harness, and by omitting the model key entirely when it resolves to nothing.</observation>
<parameter name="impact">Any prompt that instructs an agent to call spawn_agent must name the provider explicitly, or the dispatch silently becomes Claude regardless of the caller's harness. Prompt text is load-bearing wiring here, not documentation.</parameter>
<parameter name="recommendation">Keep the instruction in mcpConfig.summarizerSpawnNote — the PTY and managed prompt builders drifted once already. supervisorSkill.ts SKILL_BODY now defers to the system prompt for both provider and model rather than restating a config key.</parameter>
<parameter name="related_paths">["apps/desktop/src/main/services/mcpConfig.ts", "apps/desktop/src/main/services/supervisorSkill.ts"]
