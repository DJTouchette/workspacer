---
title: Rust cutover publication and legacy asset deletion need separate evidence
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/CUTOVER_STATUS.md
  - scripts/hub-migration.py
  - apps/desktop/electron-builder.yml
promoted: false
---

# Rust cutover publication and legacy asset deletion need separate evidence

## Observation
At SHA 1bf2f53a, CI36595813246 and release36596973339 succeeded and nightly tag/assets were published at 2026-09-29T17:02:58Z. Exact-SHA run listing had no native-client/container/native-preview runs. Electron/native/release packaging still copy services/hub/examples, so deleting the entire legacy directory would break non-Go assets. hub-migration.py ready checks ledger paths/status but does not execute tests, validate workflow SHA or prove actual legacy deletion.

## Impact
Stale migration docs and green packaging can be mistaken for parity completion; whole-directory cleanup can delete retained plugin assets.

## Recommendation
Use services/hub-rs/CUTOVER_STATUS.md gate map, final integrated revision receipts, and relocate retained examples before deleting legacy sources.
