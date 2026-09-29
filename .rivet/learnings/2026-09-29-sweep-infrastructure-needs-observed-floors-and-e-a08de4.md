---
title: Sweep infrastructure needs observed floors and executable host-case retention
date: 2026-09-29
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - services/hub-rs/tests/support/sweepguard.rs
  - apps/desktop/tests/support/sweepSourceGuard.ts
promoted: false
---

# Sweep infrastructure needs observed floors and executable host-case retention

## Observation
The original source guard mixed21 Go tally declarations with29 TS counters under a30 minimum. Rust Tally now fails if no current-population floor is observed, including discarded success/error and stale-result mutants. TS independently pins its full29-counter population and15 host references with AST scope, while a required Rust test inventory rejects removal/ignore/platform disablement. Caller audit recovered actual alias-target/global-config/four-walker/listing regressions and enabled three unnecessary Unix-only symlink gates on Windows.

## Impact
Dropping Go bookkeeping without these protections would leave both unread counters and silently lost host cases green. Linux passing does not validate newly enabled Windows branches.

## Recommendation
Keep runtime floor mutations, actual filesystem assertions and source-removal guards together; require latest-head Windows execution before cutover.
