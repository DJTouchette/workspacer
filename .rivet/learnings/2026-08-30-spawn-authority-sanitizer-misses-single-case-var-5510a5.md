---
title: Spawn authority sanitizer misses single case-variant keys on Go brain provider
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/facade.go
promoted: false
---

# Spawn authority sanitizer misses single case-variant keys on Go brain provider

## Observation
agents.spawn sanitization in services/hub/internal/bus/rpc.go strips and clamps exact lower-camel JSON keys, but the headless Go brain provider decodes spawn params with encoding/json struct tags after only rejecting duplicate case variants. A single key such as YoloGranted, ProfileGranted, Capability, Model, Effort, MCPFacade, or ToolScope can survive the sanitizer and still bind to the provider field.

## Impact
This is an authority-boundary bypass for direct bus agents.spawn callers routed to the Go brain provider: profile/yolo stamps, model/capability ceilings, toolScope, mcpFacade, and audit fields can diverge from what the bus sanitized.

## Recommendation
Reject or canonicalize authority-bearing spawn keys at the bus boundary using the same field matching semantics providers use, and add tests for single case-variant keys, not only duplicate variants.
