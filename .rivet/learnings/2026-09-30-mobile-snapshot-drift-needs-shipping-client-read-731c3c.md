---
title: Mobile snapshot drift needs shipping-client reads and actual projection values
date: 2026-09-30
confidence: high
suggested_doc: remote-mobile
related_paths:
  - services/hub-rs/tests/snapshots.rs
  - services/hub-rs/assets/web/mobile.html
promoted: false
---

# Mobile snapshot drift needs shipping-client reads and actual projection values

## Observation
The retained parity_test.go snapshot section checks12 required fields, not13, plus2 declined fields,4 usage keys,17 status-line keys and3 nesting fields. Rust snapshot fixtures alone did not link that contract to assets/web/mobile.html. New snapshots test reads the shipping Rust asset and executes compat plus durable ReplacementState enrichment/reopen, pins actual300/10080 window lengths and null monthly duration, and verifies unknown/remote rows do not acquire local nesting.

## Recommendation
Keep mobile source linkage separate from synthetic fixture equality; metadata assertions must call the production enrichment owner. A present null field is not proof that a reported duration survived projection.

## Validation
Linux snapshots target10 passed with the new shipping-mobile guard in
/tmp/workspacer-snapshot-mobile-final.log on 2026-09-30. Witness could not map
the Rust test; the explicit owning integration target supplied execution proof.
