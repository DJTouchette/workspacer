---
title: Workflow revision echoes must normalize numeric representation without resetting CAS
date: 2026-09-30
related_paths:
  - services/hub-rs/src/services/config.rs
  - services/hub-rs/tests/workflows.rs
promoted: false
---

# Workflow revision echoes must normalize numeric representation without resetting CAS

## Observation
A raw JSON workflowSelectionRevision3.0 echoed against persisted integer3 was rejected by Value equality despite the retained Go JSON comparison accepting it. Revision-only safe integral equivalence fixes that; the accepted value must then be stored as an integer or later as_u64 CAS reads could mistake it for absent0. Absent/null echoed as null remains a no-op and never invents a counter. Selected IDs/maps keep exact comparisons.

## Recommendation
Test persisted config roundtrip followed by stale0 refusal and correct3-to4 CAS, plus string/fraction/unsafe-float negatives and absent/null preservation.
