---
title: Dispatch admission and receipt tests must separate persistence from successful transport
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/remote_dispatch/receiver.rs
  - services/hub-rs/src/services/remote_dispatch/lease_audit_tests.rs
  - services/hub-rs/src/services/wakes.rs
promoted: false
---

# Dispatch admission and receipt tests must separate persistence from successful transport

## Observation
Receiver journals clone the candidate rows and commit them only after atomic persistence. Actual obstruction tests now prove failed claim writes start no worker and failed receipt writes publish no event or sequence advance; valid recovery and terminal acknowledgment survive reopening. The paired Rust fixture and desktop integration were unchanged from9bd86db6 CI where dispatch-chain16 including unknown replay passed.

## Recommendation
Keep owner/cwd/provider/expiry negatives paired with a valid unused-lease control, and test boot-idle missed-finish recovery separately from a live working-to-idle transition. Preserve unknown-execution no-replay semantics.
