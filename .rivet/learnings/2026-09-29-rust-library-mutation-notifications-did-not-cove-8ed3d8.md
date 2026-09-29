---
title: Rust library mutation notifications did not cover external editor writes
date: 2026-09-29
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/services/library_watch.rs
promoted: false
---

# Rust library mutation notifications did not cover external editor writes

## Observation
The retained Go librarywatch.go hashes redacted library.list projections for global and live-session projects every2seconds. Rust initially emitted library.changed only from explicit save/remove handlers, so external editors and agent filesystem writes never refreshed clients. A joined runtime watcher now compares the same public projections, excludes stopped/unknown/remote session directories, and publishes only an empty refresh event. It retains known directories across unavailable external inventory instead of inferring an empty fleet; owned-engine inventory uses the existing local snapshot map.
