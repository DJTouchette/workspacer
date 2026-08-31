---
title: Providerless model-selection persistence needs evidence-specific healing
date: 2026-08-31
author: Codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/windows.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/store/mod.rs
promoted: false
---

# Providerless model-selection persistence needs evidence-specific healing

## Observation
Claudemon persists canonical model identity/context-window columns and a legacy companion, but it does not persist the provider. A bare companion that equals a known selectable Claude identity is rollback evidence when the canonical window is 1M, because the correct legacy projection carries a marker. The same equality must remain valid for opaque non-Claude identities such as vendor/custom-1m. Marker-bearing canonical Claude identities are likewise malformed and should recover from the companion.

## Impact
A broad marker heuristic either lets mixed-version Claude rollback corrupt canonical state or rewrites genuine non-Claude model IDs. The restore path must heal malformed Claude pairs while preserving opaque identities byte-for-byte through hydration and the next write.

## Recommendation
Validate persisted pairs by comparing the canonical-only restore with the combined canonical/legacy restore. Reject marker-bearing canonical Claude identities and projection mismatches only when the identity is recognizable as Claude; preserve exact opaque companions. Keep mutation/load-bearing tests for bare Claude 1M, marker-bearing canonical columns, and non-Claude -1m identities.
