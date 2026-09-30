---
title: Rust launch and replacement retire private callbacks without retiring their owners
date: 2026-09-30
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/plugins/launch.rs
  - services/hub-rs/src/services/manager_replacements/native.rs
promoted: false
---

# Rust launch and replacement retire private callbacks without retiring their owners

## Observation
The retained brain launchintegration.go and replacementhost.go use a private Node pipe, but current Rust maps them to opaque broker LaunchPermit plus plugins::launch::Preparation and typed manager_replacements::NativeHost/ReplacementService. Generation-scoped Lifecycle owns credential revocation after authoritative stop; journal metadata overlays make restore publication-only rather than rewriting old Go metadata. Recovery must preserve uncertain delivery and task-first transfer, not recreate replacement.* bus capabilities.

## Recommendation
Audit current owners and their actual lifecycle/manager tests instead of searching for legacy callback method strings. Preserve no-replay and held-message/view-ack semantics when removing the Node bridge.
