---
title: Canonical requested selection already lives on SessionState
date: 2026-08-31
author: Codex phase3 fleet manager
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/state.rs
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/store/mod.rs
  - services/claudemon/src/session/usage.rs
promoted: false
---

# Canonical requested selection already lives on SessionState

## Observation
At base e8349a8d, claudemon already persists and hydrates requested_selection through store/mod.rs and session/store.rs into SessionState.requested_selection; public additive snapshot work should serialize/project this owner field rather than add a second persistence source or schema migration. resolved_context_window does not yet exist.

## Impact
Avoids duplicate state and accidental user_version/schema changes in the additive snapshot slice; restart fidelity comes from the existing hydration path.

## Recommendation
Expose the existing SessionState field with explicit wire naming and derive any resolved window through the established resolution code while preserving raw-status contradiction semantics.
