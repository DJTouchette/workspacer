---
title: Shared headless services need transport, lifecycle, and timeout parity together
date: 2026-09-11
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/headless/*
  - services/hub/cmd/brain/desktophost.go
  - services/hub/internal/bus/activity.go
  - services/hub/internal/capspec/timeouts.go
promoted: false
---

# Shared headless services need transport, lifecycle, and timeout parity together

## Observation
Wiring Fleet read APIs alone leaves an empty history: the brain must validate dispatch admission before allocation, record the actual acknowledged daemon id, and feed completion evidence into the same DispatchHistoryStore/FleetReviewStore and workflow validator. Template expansion must occur after worktree allocation so cwd names the actual execution root. Also the hub router's fixed 30s provider timeout was shorter than 150s agent-authored handoff and 5-minute worktree setup; browser timeout changes alone cannot fix that. Background desktop read APIs must be explicitly passive for the enabled Fly idle policy.
