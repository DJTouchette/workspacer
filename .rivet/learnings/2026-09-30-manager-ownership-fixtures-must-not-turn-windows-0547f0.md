---
title: Manager ownership fixtures must not turn Windows journal latency into a timeout claim
date: 2026-09-30
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/tests/manager_replacements.rs
promoted: false
---

# Manager ownership fixtures must not turn Windows journal latency into a timeout claim

## Observation
Windows a019 primary CI failed two manager_replacements ordering tests after accepted preparation with createdAt1790732260420 and updatedAt1790732261620: their shared fixture shortened preparation to1second, while real atomic journal/checkpoint filesystem work exceeded it. Same-SHA preview passed. The timeout-specific late-successor fixture already independently gates spawn completion. Non-timeout fixtures now use production Timing defaults and only shorten poll cadence; no production timeout changed.

## Recommendation
Assert ordering with operation completion and gates. Reserve accelerated deadlines for tests whose behavior is timeout; retain exact failed operation in assertions.
