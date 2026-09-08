---
title: New Agent shares its dialog with explicit task handoffs
date: 2026-09-07
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/renderer/src/components/SpawnAgentDialog.tsx
  - apps/desktop/src/renderer/src/App.tsx
promoted: false
---

# New Agent shares its dialog with explicit task handoffs

## Observation
App mounts SpawnAgentDialog for ordinary New Agent, welcome Start your first task (requireTask), and command-palette prompt recovery (defaultPrompt). Only the explicit handoff modes should render task input and send kickoffMessage. Ordinary creation must omit kickoffMessage while useAgentManager, Fleet Manager templates, and Guide retain their first-message contracts.
