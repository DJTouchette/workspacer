---
title: Brief liveness must read known rows before UI visibility filtering
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/briefs/mod.rs
  - services/hub-rs/src/services/briefs/report.rs
promoted: false
---

# Brief liveness must read known rows before UI visibility filtering

## Observation
Go brief.check deliberately counts known non-ended rows even when mode is unknown and accepts session_id when the camelCase overlay is absent. Rust previously read filtered sessions.snapshots and only sessionId, creating false stale findings for spawning rows. Briefs now reads owned local/remote session maps when an embedded engine is ready; hub-only deployments use the registered provider. Absent or malformed provider evidence produces unavailableChecks:[stale] while malformed/unreferenced checks still run, never an invented empty fleet.
