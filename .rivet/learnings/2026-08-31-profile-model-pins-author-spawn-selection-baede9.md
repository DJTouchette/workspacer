---
title: Profile model pins author spawn selection
date: 2026-08-31
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/claudeSpawn.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - services/hub/cmd/brain/handlers.go
  - apps/tui/src/bus.rs
promoted: false
---

# Profile model pins author spawn selection

## Observation
Across desktop PTY/stream, hub brain, and TUI spawn paths, a Claude profile's last executable --model value wins at the eventual CLI boundary. Before Phase 4, typed spawn metadata could be derived from the caller/default instead, so persisted model identity and context window disagreed with the process that actually ran.

## Impact
Pair-aware receivers trust canonical model identity plus context window; deriving that pair before applying profile precedence can create internally valid but false persisted state and route old and new peers to different selections.

## Recommendation
Resolve the last executable profile --model first, then derive both the canonical pair and marker-bearing legacy companion from that value. Test split and inline flag forms at every spawn boundary.
