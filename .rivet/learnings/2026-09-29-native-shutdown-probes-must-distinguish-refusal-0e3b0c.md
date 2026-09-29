---
title: Native shutdown probes must distinguish refusal from indeterminate connect failure
date: 2026-09-29
author: codex
confidence: high
suggested_doc: hub-process-supervision
related_paths:
  - apps/native/src/host.rs
  - apps/native/tests/rust_hub.rs
promoted: false
---

# Native shutdown probes must distinguish refusal from indeterminate connect failure

## Observation
Round2 Windows native and installer ownership smoke failed before rebind in verify_owned_listeners_released. The old probe allowed only one second and collapsed timeout plus unexpected OS errors into one message, so it could not distinguish delayed Windows loopback refusal from a different failure. The probe now allows five seconds, accepts only ConnectionRefused, reports timeout and OS error kind separately, and only then attempts rebind. Reachable listeners still fail immediately.

## Impact
A shutdown smoke must prove its actual reported listeners are gone, without either treating uncertainty as success or failing on an overly short refusal budget.

## Recommendation
Keep real live-listener rejection plus dropped-listener/rebind coverage, and deterministic refusal/timeout/error classification tests. Actual Windows runtime remains a CI gate; a Linux pass is not Windows timing proof.
