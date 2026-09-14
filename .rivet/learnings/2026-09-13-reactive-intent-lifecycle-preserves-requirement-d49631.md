---
title: Reactive intent lifecycle preserves requirement revisions and owns dispatch on the host
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/main/services/intentAutomationStore.ts
  - apps/desktop/src/main/services/intentAutomationRuntime.ts
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/services/intentEvidenceStore.ts
promoted: false
---

# Reactive intent lifecycle preserves requirement revisions and owns dispatch on the host

## Observation
Reactive intent activation is now stored in intent_runs (schema v6) and driven by owner-host timers, not the Work view. Saving Active or activateIntent schedules a dedicated wake-enabled manager. Requirement edits create revisions and prepare new directions; status-only saves now retain the existing revision, superseding the earlier 2026-09-13 finding that Complete invalidates current acceptance. Completion routing requires a run/revision-qualified intent-report; reports enter evidence as agent-authored reported records, never user verification. Human review acceptance completes reactive work, and changes-requested resumes it. Atomic operation claims prevent replay after ambiguous delivery; explicit inspected continuation creates a new direction while retaining the uncertain receipt.

## Recommendation
Keep native and headless adapters paired. Test activation/restart ambiguity, pause during spawn, stale report correlation, and review/evidence preservation whenever editing this controller.
