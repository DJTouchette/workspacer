---
title: A successful native preview does not certify a blocked integrated release
date: 2026-09-30
confidence: high
related_paths:
  - services/hub-rs/CUTOVER_REVIEW_7348414F.md
  - services/hub-rs/reviews/cutover-evidence-plan.json
promoted: false
---

# A successful native preview does not certify a blocked integrated release

## Observation
At7348414f exact primary, native3OS and Rust preview runs passed; Windows preview explicitly completed install/upgrade/embedded shutdown/uninstall. The earlier4fd release passed Windows/macOS standalone package smoke but failed Linux adopted Electron at the100-tool catalog floor, so Linux standalone, native package and publication were skipped. The a265 container inputs are unchanged at734, but its image stamp remainsa265 and deployment ledger explicitly requires final-head evidence.

## Recommendation
Certify individual gates using their actual executing receipts and stated scope. Do not promote a native build or preview result into a skipped release package, or relabel source-equivalent older artifacts as exact-head artifacts.
