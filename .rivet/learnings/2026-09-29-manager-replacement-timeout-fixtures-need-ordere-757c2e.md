---
title: Manager replacement timeout fixtures need ordered late-candidate receipts
date: 2026-09-29
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/tests/manager_replacements.rs
  - services/hub-rs/src/services/manager_replacements/service.rs
promoted: false
---

# Manager replacement timeout fixtures need ordered late-candidate receipts

## Observation
Windows preview CI showed interrupted-transfer phase failed before its intended recovery assertion and late-successor only one close. The shared fixture imposed1s preparation and200ms delivery despite synchronous durable filesystem writes. The production timeout path starts retained candidate cleanup concurrently with fail(): if candidate completion precedes failed-phase persistence, one subsequent fail() close is sufficient; two closes require genuinely late completion after the first close. The old350ms spawn sleep plus200ms observation sleep did not establish that ordering. Tests now use production Timing for non-timeout transfer with an asserted reparent attempt, and keep the200ms spawn timeout but gate candidate completion until failed phase/timeout reason/first close are observed, then wait for a close notification.

## Impact
A fixed elapsed sleep cannot prove candidate lifecycle ordering, especially under parallel Windows filesystem load; demanding two closes without ordering can reject safe behavior.

## Recommendation
Use explicit stage/completion barriers and preserve timeout-specific assertions; do not change production cleanup or enlarge its deadline based only on these fixtures.
