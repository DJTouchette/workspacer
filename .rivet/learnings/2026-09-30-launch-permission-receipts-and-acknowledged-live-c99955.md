---
title: Launch permission receipts and acknowledged live settings carry different facts
date: 2026-09-30
related_paths:
  - services/hub-rs/src/services/spawn_plan.rs
  - services/hub-rs/src/services/agent_lifecycle.rs
  - services/hub-rs/tests/agent_lifecycle.rs
promoted: false
---

# Launch permission receipts and acknowledged live settings carry different facts

## Observation
The retained Go launchtruth tests kept the initial permissionMode under settings while publishing livePermissionMode separately. Current Rust intentionally persists an acknowledged live mode in both settings.permissionMode and livePermissionMode and fences it by launch generation. fullAccess and bypassAvailable still describe the resolved skip flag; a requested provider bypass mode alone is not that receipt. New identity lifetimes replace old live metadata.

## Recommendation
Audit launch truth through resolve, lifecycle launch/receipt, enrichment and reopening. Do not restore stale launch settings to satisfy an obsolete floor assertion or collapse canonical model identity/context-window into a display string.
