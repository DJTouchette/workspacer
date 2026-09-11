---
title: Explicit model pins must refuse substitutions without skipping the spawn audit
date: 2026-09-11
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/mcp/workflow_dispatch.go
  - apps/desktop/src/main/services/pairedDispatch.ts
promoted: false
---

# Explicit model pins must refuse substitutions without skipping the spawn audit

## Observation
The hub's model ceiling can replace model/effort before reaching the provider. Exact user model requests need a refusal instead of that replacement, while still recording the normal spawn audit. exactModel marks the ceiling verdict denied and skips substitution; it does not return before audit recording, relax toolScope/profile/yolo grants, or fabricate a routing decision. Paired forwarding carries the flag and checks peer support.
