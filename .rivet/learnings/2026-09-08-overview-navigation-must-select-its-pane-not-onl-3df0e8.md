---
title: Overview navigation must select its pane, not only the global workspace
date: 2026-09-08
confidence: high
suggested_doc: ui-modes-manifest
related_paths:
  - apps/desktop/src/renderer/src/App.tsx
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
promoted: false
---

# Overview navigation must select its pane, not only the global workspace

## Observation
At release e443270b, SideBar brand and rail plus go-overview selected GLOBAL_WORKSPACE_ID without selecting or scrolling to its Overview pane. Recent agents shares that workspace, so its active tab survives these home actions and saved-layout restoration. OverviewPane remains registered in ScrollContainer; withGlobalWorkspace only backfills empty global tabs, not a missing Overview among other tabs. Focus only gates FleetDeck, not OverviewPane. The expanded Overview row disappeared in e005985c (2026-07-18); 4aad7498 added Recent agents and 2ba8d92e moved its Fleet entry into the retained-chat timeline, without deleting OverviewPane.

## Recommendation
Route named Overview entry points through App.openOverview: descend to piloting, open/focus the singleton overview pane in global, then scroll to its tab. Reuse renamed/split panes and recreate closed ones. Keep Fleet timeline/chat and Recent agents distinct.
