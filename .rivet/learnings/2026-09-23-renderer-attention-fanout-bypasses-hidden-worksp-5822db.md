---
title: Renderer attention fanout bypasses hidden-workspace memoization
date: 2026-09-23
confidence: high
suggested_doc: mission-control-attention
related_paths:
  - apps/desktop/src/renderer/src/contexts/AttentionContext.tsx
  - apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts
promoted: false
---

# Renderer attention fanout bypasses hidden-workspace memoization

## Observation
The current renderer already memoizes AgentWorkspaceView and compacts hidden session state to 12 turns at a 1s React flush cadence. However useSessionSnapshots.ts:90-94 still copies both global maps on every update, including unchanged ambient states. useAttentionFeed.ts:209-235 and :279 onward recomputes fingerprints and all-agent attention on map changes. AttentionContext.tsx:323-355 publishes a single flat context with snapshotBySession plus actions; its stable actions useMemo does not isolate useContext consumers. For example every mounted FleetMessageCard SessionChip at :107-109 reruns an agents.find lookup on unrelated session updates, including hidden transcript chips. The mission-control-attention context claim that action-only destructuring avoids rerenders is inaccurate.

## Impact
Fleet traffic incurs all-agent derivation and broadcasts into unrelated and hidden UI despite subtree memoization; callback fanout and collection work rise with fleet size.

## Recommendation
Coalesce promoted updates, avoid unchanged status-map writes, derive attention per changed session, and split actual action/agent-membership contexts or adopt selector subscriptions. Preserve immediate blocking approval and lifecycle events.
