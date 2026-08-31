---
title: Serde aliases never fire on the TUI's remote-fleet path — agent_from_snapshot builds Agent by hand
date: 2026-08-31
confidence: high
suggested_doc: hub-federation
related_paths:
  - apps/tui/src/federation.rs
  - apps/tui/src/types.rs
  - services/hub/cmd/brain/enrich.go
promoted: false
---

# Serde aliases never fire on the TUI's remote-fleet path — agent_from_snapshot builds Agent by hand

## Observation
apps/tui/src/types.rs `Agent` is deserialized straight from claudemon's `GET /sessions` rows (snake_case), so adding a field there with `#[serde(default, alias = "camelCase")]` covers the LOCAL path completely. It covers nothing on the federated path: apps/tui/src/federation.rs `agent_from_snapshot` CONSTRUCTS an `Agent` literal field-by-field from a `serde_json::Value` row, so any field not read out explicitly there arrives as its `Default` for every remote/peer session — silently, with no compile error beyond the struct-literal requirement being satisfied by the fields that ARE listed. The same is true of `fold_row`: a field not mentioned there is dropped whenever a sparse state-only row lands on top of a rich one, because a sparse tick omitting a field means "no claim", not "retract".

## Impact
Adding the phase-3 canonical snapshot fields (`requested_selection` / `resolved_context_window`) needed THREE edits, not one: the serde fields (local), the explicit read in agent_from_snapshot (remote), and the presence-aware merge in fold_row (sparse folds). Missing the second makes federated rows quietly field-blind; missing the third makes a value blink out mid-session on the next state tick.

## Recommendation
When adding any snapshot field to the TUI's `Agent`, do all three: serde field (with a camelCase alias for bus rows), explicit read in `agent_from_snapshot` (accept both the hub's camelCase projection and claudemon's snake_case original — the brain's compat overlay keeps both on the same row), and an `.or_else(prev)` line in `fold_row`. Never carry a peer-local filesystem path across the seam (`transcript_path` stays `None`).
