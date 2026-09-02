---
title: Model-window separation crosses a high-risk session and routing boundary
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/lib/spawnModel.ts
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - services/claudemon/src/daemon/spawn.rs
  - services/claudemon/src/store/mod.rs
  - services/hub/internal/routing/modelid.go
promoted: true
promoted_to: agent-spawn
---

# Model-window separation crosses a high-risk session and routing boundary

## Observation
The requested model feeds desktop spawn, claudemon persistence/spawn, session usage, hub capabilities/federation, and routing ceilings. The routing suffix helper intentionally preserves legacy suffixes only for comparison, while session state must retain the requested value because provider/transcript ids remove it.

## Impact
A local UI/config-only implementation would leave 1M selection unobservable in headless/federated paths or lose it before usage derives a limit.

## Recommendation
Add a versioned/optional contextWindow wire field and a shared normalization contract; migrate both desktop and brain config writers before removing suffix matching from routing.

## Disposition
Promoted into `.rivet/context/domains/agent-spawn.md`. The funnel enumeration is the durable part and is unchanged on master; the predicted failure (a UI/config-only implementation leaving 1M unobservable headlessly) was avoided — the versioned optional field it recommends now exists end to end.
