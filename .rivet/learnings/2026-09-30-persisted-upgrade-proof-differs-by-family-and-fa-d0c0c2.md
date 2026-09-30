---
title: Persisted upgrade proof differs by family and failure policy
date: 2026-09-30
confidence: high
suggested_doc: config
related_paths:
  - services/hub-rs/PERSISTED_STATE_REVIEW.md
promoted: false
---

# Persisted upgrade proof differs by family and failure policy

## Observation
Rust hub has concrete separate state proofs: CLI historical path and create-once identity, token unknown-metadata preservation, config protected bytes and two-writer merge, task/manager no-replay reopen, real fleet-review teardown/reopen, and retired intent SQLite byte preservation across two Backend starts. These do not justify one universal corruption policy: push key loss resets subscriptions, layout disk failure keeps live state, and plugin settings parse failure uses an empty overlay. PERSISTED_STATE_REVIEW.md distinguishes executed scoped receipts, read owning assertions and missing historical/failure evidence without marking a gate.

## Recommendation
Certify reviewed per-family fixture/restart/identity-loss evidence against final platform receipts. Do not infer failure policy or historical-schema coverage from path mappings, and do not invent a universal all-store or downgrade requirement.

## Source verification refinement
Profiles already have a malformed-byte list/add refusal and unchanged-byte
assertion inside the fractional-weight regression. Plugin empty-overlay fallback
matches the retained Go readSettingsOverlay exactly. Claudemon's unchanged store
owner includes old-schema, failed-step rollback, lost-version repair and actual
file reopen tests; hub compilation is not evidence those dependency tests ran.
