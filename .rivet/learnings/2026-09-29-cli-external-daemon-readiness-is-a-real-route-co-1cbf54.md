---
title: CLI external-daemon readiness is a real route-contract caller
date: 2026-09-29
promoted: false
---

# CLI external-daemon readiness is a real route-contract caller

## Observation
The Rust hub-only launcher readiness probe validates the external daemon URL through ExternalDaemon then performs its own base.join("health") GET. The cross-stack claudemon route inventory must enumerate cli/readiness.rs with a relative URL-join scanner and a floor of one, rather than classify its ExternalDaemon mention as a non-caller. Mutation tests require /health to remain served and this caller to remain explicitly enumerated.
