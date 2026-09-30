---
title: Deleted reference tests assert sealed capture identity, not missing-file diagnostics
date: 2026-09-30
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - tools/capability-source-check/tests/policy.rs
promoted: false
---

# Deleted reference tests assert sealed capture identity, not missing-file diagnostics

## Observation
Post-removal CI exposed a policy test that correctly rejected a tampered historical scanner digest but still expected a present-source mismatch message. The test now requires exact parsed-reference-versus-sealed-capture rejection and restores each mutation before the next control. Missing dangerous binding and wrong population assertions remain specific; existing isolated reference_capture tests retain present-byte mismatch coverage.
