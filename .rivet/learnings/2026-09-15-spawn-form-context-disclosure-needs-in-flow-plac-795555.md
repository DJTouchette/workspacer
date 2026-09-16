---
title: Spawn form context disclosure needs in-flow placement at narrow widths
date: 2026-09-15
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/renderer/src/components/SpawnAgentDialog.tsx
  - apps/desktop/src/renderer/src/components/SpawnAgentDialog.css
  - apps/desktop/src/renderer/src/components/ModelContextPopover.tsx
promoted: false
---

# Spawn form context disclosure needs in-flow placement at narrow widths

## Observation
ModelContextPopover uses a 260px absolutely positioned panel aligned to its trigger's right edge. Moving it into a wrapping F-line option strip can place that edge near the left viewport edge and overflow narrow screens. SpawnAgentDialog locally lays this shared disclosure out in flow; other surfaces retain their floating layout. The pre-existing spawn header on this branch is a dynamic AgentLogo (30px in a 64px circle), title, subtitle and accent glow, not a Workspacer wordmark.
