---
title: An updated launch provider must reject model substitutions from adopted older hubs
date: 2026-09-11
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# An updated launch provider must reject model substitutions from adopted older hubs

## Observation
Desktop/server process adoption can leave a newer provider behind an older hub that ignores exactModel and still clamps model/effort. Both launch providers now reject exact-model requests when the existing hub-authored escalationScrubbed receipt reports a model/capability/effort/context substitution, before launching any worker. Permission-only toolScope clamps remain allowed. This also makes the brain's exactModel capability advertisement reliable behind an older hub.
