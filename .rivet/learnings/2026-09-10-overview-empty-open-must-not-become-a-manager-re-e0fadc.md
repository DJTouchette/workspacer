---
title: Overview empty open must not become a manager request
date: 2026-09-10
confidence: high
suggested_doc: modules/fleet-manager
related_paths:
  - apps/desktop/src/renderer/src/components/FleetManagerHero.tsx
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
promoted: false
---

# Overview empty open must not become a manager request

## Observation
At 5bfc7108 FleetManagerHero disabled empty input independently of advisory provider readiness. The palette sent a canned inventory ask through the same App event, rather than only focusing. Empty opening must bypass request capture and message delivery, while fresh managed sessions need doctrine in deferred instructions for their first real composer turn. Manager selection before onSessionReady completes hides the Overview draft on capture or delivery failure.

## Impact
Do not diagnose an unsupported probe as a launch gate from an empty-input screenshot, or create phantom request records to open a manager.

## Recommendation
Keep empty open and typed ask on the shared App handler; retain real runtime preflight and focus existing managers before launch gates. Verify the new regression tests on hosted CI; they have not been run locally due to the OOM restriction.
