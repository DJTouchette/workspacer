---
title: Headless blocked wakes are fleet-wide while finish wakes stay owned
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/brain/finishwake.go
  - services/hub/cmd/brain/blockwake.go
  - apps/desktop/src/main/services/pairedDispatch.ts
promoted: false
---

# Headless blocked wakes are fleet-wide while finish wakes stay owned

## Observation
services/hub/cmd/brain/finishwake.go routes remote-origin workers through their dispatch return channel before local parent lookup, so normal finish/progress wakes do not reach an unrelated local manager. However finishWatcher.observe feeds blockWatcher.onEdge first, and blockwake.go intentionally broadcasts a surviving blocked state to every live isWakeTarget on that execution hub. A local Fly manager can therefore hear a remote desktop worker's blocked notification without owning its result.

## Impact
Coexistence guidance must distinguish blocked notifications from ownership/result routing; claiming all remote updates are excluded is too broad.

## Recommendation
For a live acceptance test assert desktop-specific progress/finish routing and explicitly document the expected cross-manager blocked broadcast.
