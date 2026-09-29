---
title: Brief archive optional bounds follow numeric and null semantics before writes
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/briefs/mod.rs
promoted: false
---

# Brief archive optional bounds follow numeric and null semantics before writes

## Observation
Legacy brief.archive optional float64 bounds treat null as absent and accept integral numeric spellings such as2.0; Rust Value::as_u64 rejected those. Briefs now validates exactly-one bound before creating lock/directories, accepts finite nonnegative integral values within host signed-integer range, and retains count/keep meaning. A private CAS callback seam tests an actual outside write between reads and proves archive side effects occur once after retry.
