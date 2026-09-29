---
title: Legacy SSE must bind each session to its exact credential and retain raw validation
date: 2026-09-28
promoted: false
---

# Legacy SSE must bind each session to its exact credential and retain raw validation

## Observation
The Rust legacy SSE adapter drives rmcp through typed channel Transport, inserts freshly authenticated HTTP Parts into every request, and binds the session registry to full token fingerprint plus static-host class. Idle streams revalidate scoped tokens each second. Raw routing preference validation must run before SDK JSON deserialization and enqueue a typed SSE result with HTTP202; returning a direct HTTP200 JSON body breaks legacy transport framing. Supervisor parity requires sidecar.running/healthy/unhealthy/crashed/stopped topics, source supervisor, pid on running/health, err on crashes; the initial failed health probe emits nothing until a prior healthy transition.
