---
title: Migration completion indicator is a shared build milestone not live readiness
date: 2026-09-30
confidence: high
related_paths:
  - services/hub-rs/src/lib.rs
  - services/hub-rs/src/main.rs
  - services/hub-rs/src/mcp.rs
  - services/hub-rs/tests/mcp.rs
promoted: false
---

# Migration completion indicator is a shared build milestone not live readiness

## Observation
The cutover indicator routes both low-level wks-hub startup JSON and actual MCP health migrationComplete through one compile-time MIGRATION_COMPLETE constant. MCP integration now asserts the live HTTP health value. hubConnected, pluginCatalogReady and launchReady remain separate live state; completing migration must not imply every provider is already registered.

## Recommendation
Apply the true milestone only after completed deletion gates and CI. The root review approved application after post-removal revision96e2568a passed primary, native-client and container workflows and all14 gates were verified. The indicator has a real HTTP integration assertion and must pass normal CI before publication; formatting alone is not execution evidence.
