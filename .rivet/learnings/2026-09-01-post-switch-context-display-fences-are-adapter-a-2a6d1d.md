---
title: Post-switch context display fences are adapter and transport specific
date: 2026-09-01
author: codex
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/providers/claude_stream.rs
promoted: false
---

# Post-switch context display fences are adapter and transport specific

## Observation
Claude stream-json and Codex app-server both retain a cached context numerator/window in UsageAcc across model-only ticks, but Claude PTY does not use that ingress. Codex explicitly marks a fresh runtime last/window pair, while Claude can still deliver a correlated predecessor result after the bounded model-label fence expires, so Claude stream requires a pair compatible with the accepted successor selection.

## Impact
A provider-only blanket fence breaks Claude PTY compatibility, while treating every fresh Claude pair as successor evidence can divide a fresh predecessor numerator by the wrong post-switch window or republish stale display telemetry after tick three.

## Recommendation
Keep the policy explicit by provider and transport: Codex releases on any explicit fresh runtime pair, Claude stream releases only on a fresh selection-compatible result pair, and unproven adapters remain unfenced. Pin the distinction with full-suite PTY, Codex, and real claude_stream regressions.
