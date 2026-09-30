---
title: Literal short-circuit tracing removes dead effort bindings without hiding dynamic reads
date: 2026-09-30
related_paths:
  - tools/capability-source-check/src/trace.rs
  - tools/capability-source-check/tests/bindings.rs
  - services/hub-rs/src/services/live_controls.rs
promoted: false
---

# Literal short-circuit tracing removes dead effort bindings without hiding dynamic reads

## Observation
A same-current-source comparison with the prior tracer removed13 false on bindings from sessions validators plus exactly3 formerly counted dangerous bindings: claude.setPermissionMode.effort, claude.handoffBrief.effort and claude.handoffAgentBrief.effort. Those reads occur only in live_controls.rs guards whose literal method must equal claude.setEffort or claude.setModel, so Rust never evaluates their RHS for those3 other methods. Real setEffort/setModel effort reads remain. Known None prevents impossible if-let Some reads; Some(false) still matches, and unknown keys/guards remain unresolved.

## Recommendation
Keep source population changes tied to exact method/field deltas and literal control-flow evidence. Do not exempt entire helpers or suppress unresolved dynamic inputs.
