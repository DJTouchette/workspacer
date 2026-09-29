---
title: Federation migration preserves owner facts semantically and retires the unused peer argv flag
date: 2026-09-29
suggested_doc: hub-federation
related_paths:
  - services/hub-rs/reviews/federation.json
  - services/hub-rs/tests/federation.rs
  - services/hub-rs/src/runtime/federation_routing_tests.rs
promoted: false
---

# Federation migration preserves owner facts semantically and retires the unused peer argv flag

## Observation
The retained Go ownerselection test asserts RawMessage byte equality for requestedSelection, resolvedContextWindow, usage and contradictory provider statusLine. Rust Event.data is serde_json::Value, so the correct preserved contract is whole-value equality including explicit null and absence, not JSON key order/whitespace. Added this exact payload to a real two-hub forwarding test. Controller parity additionally requires exact unchanged-link identity and changed/removed-link retirement; successful reconnect alone is insufficient. Source/recon plus docs/deploy/script sweeps found no shipped consumer of Go -peer, only original component parser/history; current Rust --peers-file and owner config APIs replace it.

## Impact
Without independent assertions, destination sanitation can hide missing source policy and semantic model facts can be mistaken for serialization formatting guarantees.

## Recommendation
Keep the two complementary source-only and destination-only routing tests, explicit JSON semantic exception and documented peers-file CLI replacement in migration review.
