---
title: Sanitizer drift regressions must distinguish source and destination passes
date: 2026-09-29
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/admission.rs
  - services/hub-rs/src/runtime/bus_audit_tests.rs
promoted: false
---

# Sanitizer drift regressions must distinguish source and destination passes

## Observation
A real two-hub sanitizer test can falsely pass if only the destination sanitizes. The Rust test-only third sanitizer therefore records identity.federated for each pass, requiring [false] locally and [false,true] for a qualified call in addition to stripping the secret and preserving unrelated fields. This exercises shared admission without adding a production method or mutable global test registry.

## Impact
A sanitized destination result alone does not prove secrets were removed before crossing the peer hop.

## Recommendation
Preserve the source and destination provenance assertion when adding new dispatch paths.
