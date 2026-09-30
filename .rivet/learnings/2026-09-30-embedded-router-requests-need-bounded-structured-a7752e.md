---
title: Embedded router requests need bounded structured concurrency
date: 2026-09-30
suggested_doc: claudemon-http-api
related_paths:
  - services/claudemon/src/daemon/embedded.rs
  - services/claudemon/tests/support/embedded_control.rs
promoted: false
---

# Embedded router requests need bounded structured concurrency

## Observation
The actual serve_commands regression held one router request while a real HTTP fast sibling succeeded; the embedded fast sibling remained blocked until release, proving the serial dispatch bottleneck. The owner now polls eight FuturesUnordered entries with no per-request spawned task, retaining queue64 and both thirty-second bounds. Tests prove queue pressure, skipped closed queued calls, continued started effects after caller cancellation, draining channel closure, owner-abort future cleanup and exact timeout uncertainty. Control5+response2 passed, then full claudemon library905 passed with4 real-home/network checks ignored. An initial debug2 direct-manifest build exhausted disk before any test and is not failure-reproduction evidence.

## Impact
An in-process adapter must not serialize every API behind one slow model lookup, and parallelism must not create detached shutdown work.

## Recommendation
Keep dispatch futures owned and bounded, preserve uncertain outcomes, and use barrier-based causal tests rather than raising timing budgets. Use stripped local profiles explicitly when invoking the claudemon manifest with the shared cache.
