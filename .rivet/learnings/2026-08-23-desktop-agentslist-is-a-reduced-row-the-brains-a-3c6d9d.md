---
title: Desktop agents.list is a REDUCED row; the brain's agents.list returns full enriched snapshots
date: 2026-08-23
author: session: parentSessionId-on-agents.list
confidence: high
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/enrich.go
  - services/hub/cmd/brain/handlers.go
  - services/hub/internal/authtoken/authtoken.go
promoted: false
---

# Desktop agents.list is a REDUCED row; the brain's agents.list returns full enriched snapshots

## Observation
The two providers of `agents.list` do not return the same shape. The desktop (apps/desktop/src/main/services/hubCapabilities.ts) hand-builds a small row (sessionId/cwd/state/model/context/cost/pending*/lastActivity). The headless brain (services/hub/cmd/brain/handlers.go) answers `agents.list` and `sessions.snapshots` with the SAME value — `visibleSnapshots`, i.e. full claudemon rows run through enrichAndCompat — so brain rows have carried label / parentSessionId / isSupervisor (enrich.go enrichSnapshot) all along, pinned by parity_test.go TestEnrichSnapshotCoversMobileNestingFields because /m nests the fleet on them. So "adding a field to agents.list" is desktop-only work in practice, and the drift direction is the desktop LAGGING the brain, not leading it. Corollary for disclosure review: a field already on the desktop's `sessions.snapshots` (which spreads the whole snapshot through compactClaudeSnapshotForBackground) is already readable at the VIEW tier, because authtoken.go viewMethods admits agents.list and sessions.snapshots equally — putting that same field on the lighter row widens no boundary.

## Impact
Disclosure reviews of `agents.list` that only read hubCapabilities.ts conclude a field is new to the tier when it usually is not; and a "brain parity" worry is often already satisfied.

## Recommendation
Before widening agents.list, check (a) whether the brain already emits the field via enrichSnapshot/compatSnapshot, and (b) whether desktop sessions.snapshots already ships it — both are the same view tier. The desktop agents.list tests use expect.objectContaining, so adding a field does not break them.
