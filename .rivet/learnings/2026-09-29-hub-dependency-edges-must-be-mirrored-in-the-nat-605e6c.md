---
title: Hub dependency edges must be mirrored in the native consumer lockfile
date: 2026-09-29
confidence: high
related_paths:
  - apps/native/Cargo.lock
  - services/hub-rs/Cargo.toml
promoted: false
---

# Hub dependency edges must be mirrored in the native consumer lockfile

## Observation
Adding yaml-rust as a direct hub dependency left apps/native/Cargo.lock missing its existing-package edge. All hub Windows tests passed, then native --locked refused to build in run36642609900/job109658136093. The one-line native edge correction passed locked Linux and Windows dependency resolution with cargo tree; no versions or other dependency edges changed.

## Recommendation
When changing the hub Cargo manifest, inspect every path-dependent consumer lockfile and run locked resolution for native features before pushing. A cached package and an unchanged package version do not remove the consumer-lock requirement.
