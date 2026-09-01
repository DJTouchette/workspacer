---
title: Context-less usage ticks must defer to SessionStore fencing
date: 2026-09-01
author: codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/providers/mod.rs
promoted: false
---

# Context-less usage ticks must defer to SessionStore fencing

## Observation
UsageAcc retains its last correlated context_health and includes it on later model/cost-only status lines with context_health_updated=false. After a model boundary clears SessionStore's health, the bounded model fence can expire and a naive reconciler will stamp that cached predecessor sample with the successor context_telemetry_epoch.

## Impact
A stale 180000/200000 sample can be laundered into the new epoch and fire a post-switch health watch before the new runtime has reported current usage.

## Recommendation
Treat context_health_updated as the authority bit: on false, SessionStore must replace inbound context_health with its own stored fenced value, including None; only true may validate and stamp a new correlated observation.
