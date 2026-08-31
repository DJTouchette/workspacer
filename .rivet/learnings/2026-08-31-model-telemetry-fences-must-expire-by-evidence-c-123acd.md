---
title: Model telemetry fences must expire by evidence count, not provider-name equality
date: 2026-08-31
author: codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/state.rs
  - services/claudemon/src/session/store.rs
  - apps/desktop/src/main/services/claudeSessionStore.ts
promoted: false
---

# Model telemetry fences must expire by evidence count, not provider-name equality

## Observation
The Phase 5 owner fence is re-armed from every persisted canonical selection and clears only when a later status frame's display/window is compatible. Claude status display names are not guaranteed to normalize to the requested identity (for example Haiku-style names), so equality-only release can permanently suppress truthful telemetry across restart. A safe fence needs a persisted-independent finite evidence budget: suppress only the first bounded number of incompatible post-accept frames, clear immediately on compatible telemetry, and let later divergence through.

## Impact
An unbounded or hydration-rearmed fence can hide the real provider model/window forever; the desktop mirror can repeat the same failure even after the daemon becomes truthful.

## Recommendation
Represent the fence as a small remaining-frame budget owned only by locally accepted switches. Hydration may re-arm with the same maximum budget, never extend it. Tests should pin repeated stale suppression, early match, mismatch exhaustion, restart, and later truthful divergence.
