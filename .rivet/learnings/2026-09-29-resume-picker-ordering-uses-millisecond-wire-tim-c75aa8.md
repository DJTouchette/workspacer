---
title: Resume picker ordering uses millisecond wire timestamps
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/discovery.rs
promoted: false
---

# Resume picker ordering uses millisecond wire timestamps

## Observation
Go listClaudeSessionsForDir sorts formatted millisecond timestamps stably after os.ReadDir filename order; the Rust discovery list previously sorted SystemTime nanoseconds first, reordering sessions with equal wire timestamps. The Rust comparator now compares timestamp_millis then filename. Boundary fixture gives a.jsonl and z.jsonl different submillisecond mtimes and asserts their shared wire timestamp retains filename order. Separate exact Go clipping vectors pin 99 ASCII plus é and odd/even astral clipping.
