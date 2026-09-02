---
title: Model marker remains a transport-dependent context-window carrier
date: 2026-08-30
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/shared/modelContextWindows.ts
  - apps/desktop/src/main/services/modelUsage.ts
  - services/hub/cmd/brain/windows.go
  - services/claudemon/src/providers/mod.rs
  - services/claudemon/src/session/windows.rs
promoted: true
promoted_to: session-lifecycle
---

# Model marker remains a transport-dependent context-window carrier

## Observation
Current code has separate desktop TS, headless brain Go, and claudemon Rust window resolvers. `[1m]`/`-1m` suffixes still determine requested context at spawn and can out-rank a 200K status-line report; live usage must retain requested model/context before provider observation. This makes model strings semantically load-bearing across spawn, restore, and federation, not merely display aliases.

## Impact
A partial, layer-only migration can regress a live 1M session to a 200K denominator after a status or snapshot update, especially across old peer versions.

## Recommendation
Introduce an explicit `{model, contextWindow}` selection field with monotonic/authority-aware reconciliation, then retain suffix parsing only as bounded ingress compatibility. Contract-test desktop, brain, claudemon and federated snapshot skew.

## Disposition
Promoted into `.rivet/context/domains/session-lifecycle.md`. The observation held; the RECOMMENDATION has since shipped, so it is recorded as the current shape (canonical `{model, contextWindow}` selection + marker as bounded ingress compatibility, pinned by `contracts/model-context-windows.json`'s `selectionCases`/`claudeArgvCases`) rather than as work to do.
