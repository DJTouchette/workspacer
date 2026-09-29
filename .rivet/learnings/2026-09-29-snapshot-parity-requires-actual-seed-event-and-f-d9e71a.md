---
title: Snapshot parity requires actual seed event and fallback routes
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/sessions.rs
  - services/hub-rs/tests/snapshot_routes.rs
promoted: false
---

# Snapshot parity requires actual seed event and fallback routes

## Observation
Pure compat tests cover sparse owner-selection metadata but cannot prove that initialization, live typed updates and uncached singular reads all use that projection. The existing typed-lag regression is unchanged since c0f40add, with exact Windows CI execution verified against source SHA68d5225f49983926f8217e56d9d4ef03d922b15137b866f75fbf4f539f7ec61b.

## Recommendation
Use a single-engine integration executable with persisted selection and an empty retained row for uncached fallback; enforce the portable fixture executed-case floor.
