---
title: Claude live effort goes through /effort, not /model; native model chip uses setModel then setEffort
date: 2026-10-05
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - apps/native/src/backend.rs
  - apps/native/src/ui/features.rs
  - services/claudemon/src/providers/claude_stream.rs
promoted: false
---

# Claude live effort goes through /effort, not /model; native model chip uses setModel then setEffort

## Observation
claudemon POST /sessions/:id/model forwards effort to managed drivers, but claude_stream drops ModelSwitch.effort (debug log only), while the hub still records settings.effort on success. Hub claude.setEffort routes per provider: Claude gets the '/effort <level>' slash command through the message path, Codex gets /model with effort only (thread/settings/update). Desktop liveEffort.ts does the same. Native Change model therefore sends claude.setModel without effort, then claude.setEffort, and only fields that changed.

## Impact
Sending effort inside claude.setModel for a Claude session silently does nothing yet marks the snapshot as changed.

## Recommendation
Use claude.setEffort for effort on every provider; never piggyback effort on claude.setModel for Claude.
