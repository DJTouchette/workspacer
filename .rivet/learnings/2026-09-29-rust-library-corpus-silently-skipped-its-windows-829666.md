---
title: Rust library corpus silently skipped its Windows symlink refusals
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/library.rs
  - services/hub-rs/tests/support/sweepguard.rs
promoted: false
---

# Rust library corpus silently skipped its Windows symlink refusals

## Observation
selected_library_item_directories_match_corpus in services/hub-rs/src/services/library.rs previously continued every needsSymlinks row on Windows and asserted only fixture length. It now attempts native directory symlinks, records unavailable setup by case name, and requires all seven cases with three executed accepts/four refusals. A failure-injection fixture proves unavailable symlink privilege fails the denominator. tests/files.rs and stores.rs now use the shared test-only Tally for execution floors; Go process-global GateCounter and Root reader migration remain pending, documented in SWEEP_GUARDS.md.
