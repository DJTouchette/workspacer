---
title: The routing matrix's `fresh` flag is now enforced at the spawn gate, not just reported
date: 2026-08-30
confidence: high
related_paths:
  - services/hub/internal/routing/fresh.go
  - services/hub/internal/routing/ceiling.go
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/hub/routingceiling.go
  - services/hub/scripts/routing-limit-harness.mjs
promoted: false
---

# The routing matrix's `fresh` flag is now enforced at the spawn gate, not just reported

## Observation
`fresh: true` on a profile entry (reviewer, deep_reviewer, frontier_plus in every shipped profile) used to ride on the routing.select answer and be acted on nowhere. `resumeSessionId` is on the spawn wire (cmd/brain/handlers.go:577, `resume := p.ResumeSessionID != ""` at :791), so a reviewer could be pointed at the implementer's own session and inherit the reasoning chain it was spawned to grade. As of this change routing.Matrix.CheckSpawn carries a FRESHNESS arm (internal/routing/fresh.go) that sets CeilingVerdict.ResumeRefused, and internal/bus sanitizeSpawnParams REFUSES the call rather than stripping the field. Three design points that are not obvious from the code: (1) the arm runs BEFORE CheckSpawn's `if key == ""` early return, because a matrix with no `ceilings:` block must still keep a reviewer independent; (2) a role is read at its STRONGEST, so the base capability under `roles:` and every capability a `mode_shifts:` entry would move that role onto are all checked, since the gate cannot know which mode the caller decided under; (3) it keys off the DECLARED role/capability with no provenance mechanism, deliberately unlike the capability ceiling which refuses to trust `decisionId`, because refusing a resume only ever gives a caller LESS and a caller that lied about its role could have gained the same by declaring no role at all.

## Impact
Closes the "setting written and never read" class for `fresh`. Also means a dispatcher that legitimately wanted to continue a session must now stop labelling that work as a review role: the spawn fails with an error naming the session id rather than silently starting a new one.

## Recommendation
A resolver injected via SetSpawnCeiling now answers two questions, not one (ceiling clamp + freshness refusal). If you add a third routing judgement to the spawn path, extend that same resolver rather than adding a second hook: methodSanitizers is the single dispatch table for call() and federatedCall(), which is the only reason the federated hop is covered by construction. Verified by internal/federation/spawnsanitizer_test.go against two real hubs and by the routing harness (76 active checks) against a live hub.
