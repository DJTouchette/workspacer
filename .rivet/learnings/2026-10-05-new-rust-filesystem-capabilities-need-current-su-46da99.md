---
title: New Rust filesystem capabilities need current surface and cross-language authority records
date: 2026-10-05
confidence: high
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - contracts/backend-capabilities.json
  - apps/desktop/tests/fixtures/capability-parameter-policy.json
  - apps/desktop/src/main/services/capabilityRegistration.test.ts
promoted: false
---

# New Rust filesystem capabilities need current surface and cross-language authority records

## Observation
fs.compareWrite was registered in the Rust file service and relay but absent from the required backend full/catalog surface and TS registration/parameter policy, so native tests passed while three desktop CI guards failed. Keep hub-vocabulary.json and the original Go capture sealed; declare current additions in backend-capabilities.json, classify acting methods in the explicit EXTRA authority registry, and put non-vocabulary helper fields in sourceParameterDecisions. The TS inline scanner sees69 arms/47 dangerous bindings; Rust AST separately follows files::write and proves contents, expected, force and path.
