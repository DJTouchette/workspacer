---
title: Codex app-server plans use turn/plan/updated
date: 2026-08-26
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/session/state.rs
promoted: false
---

# Codex app-server plans use turn/plan/updated

## Observation
The installed Codex 0.150.1 app-server schema exposes an authoritative `turn/plan/updated` notification with `plan[].status` values including camelCase `inProgress`. claudemon's Codex provider currently only extracts plan-shaped items from `item/started` and `item/completed`, and the shared `PlanStatus::from_wire` parser does not recognize `inProgress`.

## Impact
Live Codex plans can be ignored entirely, or their active step can render as pending if routed through the generic plan parser.

## Recommendation
Handle `turn/plan/updated` in `services/claudemon/src/providers/codex.rs`, widen `PlanStatus::from_wire` for Codex's camelCase status, and avoid deriving full plans from experimental `item/plan/delta` chunks.
