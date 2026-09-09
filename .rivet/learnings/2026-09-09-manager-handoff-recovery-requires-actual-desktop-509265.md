---
title: Manager handoff recovery requires actual desktop capability ownership
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacement.ts
  - apps/desktop/src/main/services/managerReplacementState.ts
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
  - apps/desktop/src/main/services/fleetReviewStore.ts
promoted: false
---

# Manager handoff recovery requires actual desktop capability ownership

## Observation
Desktop-local replacement is gated by the hub registered acknowledgement for agents.spawn, agents.sendMessage, agents.reparent and fleetWorkflows.request, in addition to remote-server and adopted-hub checks. A connected owned hub alone does not prove desktop ownership: first-registration-wins can withhold methods. The manager-replacements JSON journal records launch/parent metadata absent from claudemon persistence, correlated in-flight message IDs, held wakes and validated artifact bytes. Explicit retry records are separate from the original uncertainty. Task adoption refuses dispatchReservation inside the task JSON lock before any task or worker change; review allocation custody and immutable capture readers also need succession handling.

## Impact
Replaying a journal against a headless owner, losing an in-flight request ID, or moving a reserved task can silently split ownership or duplicate manager work.

## Recommendation
Keep manager replacement IPC local-only, park successor without firstMessage/resume, test actual spawn/grants/store/wake integration, and preserve explicit recovery-required after missing acknowledgements. Startup-ping integration must exclude replacementSessionId spawns. Keep task JSON and review-history custody changes in the same host transition.
