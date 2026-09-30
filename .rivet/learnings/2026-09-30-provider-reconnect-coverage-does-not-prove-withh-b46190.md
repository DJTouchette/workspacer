---
title: Provider reconnect coverage does not prove withheld-method registration recovery
date: 2026-09-30
confidence: high
suggested_doc: hub-federation
related_paths:
  - services/hub/cmd/brain/reregister_test.go
  - services/hub-rs/src/provider_relay/mod.rs
  - services/hub-rs/src/provider_relay/tests.rs
promoted: false
---

# Provider reconnect coverage does not prove withheld-method registration recovery

## Observation
Retained reregister_test.go evicts a DIFFERENT stale owner after the brain was refused, requiring recovery without reconnect. Current Rust test evicts the relay itself and proves reconnect/no mutation replay, a different scenario. Relay session currently captures accepted methods immutably after initial ack, ignores later registered frames, and has no registration retry timer. A real stale-predecessor socket probe is still needed before certifying the pending migration row; no production edit or runtime failure is claimed from this read-only audit.


## Reproduction and correction
The real socket test subsequently reproduced the gap: 11 tests passed and the
stale-predecessor test failed after seven seconds, while config.get retained the
same relay provider ID and brain.info stayed absent. The fix retains a
connection-owned five-second registration interval while grants are missing and
processes each negotiated acknowledgement into an offered-set intersection.
The same test now passes without reconnect; duplicate partial acknowledgements
cannot advertise unoffered methods, and actual socket retirement proves shutdown
ends the retry owner. All 13 selected tests passed in
/tmp/workspacer-provider-recovery-after.log on 2026-09-30.
