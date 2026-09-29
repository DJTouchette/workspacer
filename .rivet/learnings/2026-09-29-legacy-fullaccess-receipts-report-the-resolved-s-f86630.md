---
title: Legacy fullAccess receipts report the resolved skip flag independently of provider mode
date: 2026-09-29
confidence: high
related_paths:
  - services/hub-rs/src/services/spawn_plan.rs
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# Legacy fullAccess receipts report the resolved skip flag independently of provider mode

## Observation
Go handlers.go sets spawnResult.fullAccess and managed Yolo from p.skip, while permissionMode is a separate field. Rust spawn_plan preserves that behavior: mode-only bypassPermissions yields provider bypass mode but fullAccess remains the resolved skip flag. The new grant-parity test initially assumed both were true and failed; exact source review corrected that assumption. Missing/true/false/null obsolete grant stamps leave all 24 provider plans unchanged.

## Recommendation
Do not infer a grant or silently change the wire contract from the permission-mode spelling. Any future fullAccess semantic correction must update all provider/client owners together.
