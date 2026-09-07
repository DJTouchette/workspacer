---
title: Fleet runbook timestamps describe current worker snapshots, not an event history
date: 2026-09-06
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/components/FleetDeck.tsx
  - apps/desktop/src/renderer/src/components/FleetTimeline.tsx
  - apps/desktop/src/renderer/src/components/AgentCard.tsx
promoted: false
---

# Fleet runbook timestamps describe current worker snapshots, not an event history

## Observation
Fleet layout C sorts one overview row per worker by snapshot.lastActivity descending, with missing/invalid times last and stable ties. AgentWorkspace.manager alone fixes manager rows above workers; attention state affects labels and actions rather than time ordering. Timeline AgentCard presentation keeps its original useSessionChatController send and pending ownership, and FleetChatDestination still moves the existing pane.

## Recommendation
Do not synthesize events or summaries from activity order, infer managers from names, or create a transcript store for the timeline. Keep unknown time and requested model fallbacks explicit.
